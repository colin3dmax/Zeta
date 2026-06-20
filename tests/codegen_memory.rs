// Native backend memory management: array locals are freed at scope exit on the
// fall-through path (so a loop body reclaims its per-iteration allocations).
// Value semantics deep-copies arrays at every binding, so each array local
// uniquely owns its capacity-headed buffer — freeing the dead local at scope
// exit is sound (no use-after-free; the differential result must stay correct).
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_memory
#![cfg(feature = "llvm")]

use zeta::ast::Item;
use zeta::runtime::Value;

fn structs_of(source: &str) -> Vec<zeta::ast::StructDecl> {
    zeta::parse_source(source)
        .expect("parse")
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(decl) => Some(decl.clone()),
            _ => None,
        })
        .collect()
}

/// Native result must equal the interpreter oracle (catches use-after-free).
fn check(source: &str) -> i64 {
    let structs = structs_of(source);
    let program = zeta::lower_source(source).expect("lower");
    let oracle = match zeta::runtime::run_mir(&program).expect("interpreter") {
        Value::Int(n) => n,
        other => panic!("expected Int, got {other:?}"),
    };
    let native = zeta::codegen::jit_run_i64(&program, &structs, "main").expect("native JIT");
    assert_eq!(native, oracle, "native/interpreter divergence\n{source}");
    oracle
}

fn ir_of(source: &str) -> String {
    let structs = structs_of(source);
    let program = zeta::lower_source(source).expect("lower");
    zeta::codegen::emit_llvm_ir(&program, &structs).expect("emit ir")
}

#[test]
fn loop_body_array_is_freed_each_iteration() {
    // `ys` is declared in the while body → its buffer is freed at the end of
    // each iteration. The IR must contain a `free`, and the result stays correct.
    let src = "\
fn main() -> Int {
  let mut total: Int = 0;
  let mut i: Int = 0;
  while i < 100 {
    let ys: IntArray = [i, i + 1, i + 2];
    total = total + ys[0] + ys[2];
    i = i + 1;
  }
  return total;
}";
    // sum over i in 0..100 of (i) + (i+2) = sum(2i+2) = 2*4950 + 200 = 10100.
    assert_eq!(check(src), 10100);
    let ir = ir_of(src);
    assert!(
        ir.contains("call void @free("),
        "expected a free() in the IR; none found:\n{ir}"
    );
}

#[test]
fn nested_block_array_is_freed() {
    let src = "\
fn main() -> Int {
  let mut acc: Int = 0;
  if acc == 0 {
    let xs: IntArray = [10, 20, 30];
    acc = acc + xs[1];
  }
  return acc;
}";
    assert_eq!(check(src), 20);
    assert!(ir_of(src).contains("call void @free("));
}

#[test]
fn returned_array_is_not_freed_no_use_after_free() {
    // An array built in a function and returned must NOT be freed before the
    // return (it escapes); the caller still reads it correctly.
    let src = "\
fn make(n: Int) -> IntArray {
  let xs: IntArray = [n, n + 1, n + 2];
  return xs;
}
fn main() -> Int {
  let a: IntArray = make(7);
  return a[0] + a[1] + a[2];
}";
    assert_eq!(check(src), 24);
}

#[test]
fn reassigning_array_local_frees_old_buffer() {
    // v2: overwriting a simple array local frees the previous buffer (the new
    // value is already a fresh/copied buffer, so freeing the old is safe).
    let src = "\
fn main() -> Int {
  let mut a: IntArray = [1];
  a = [2, 3];
  return a[0] + a[1];
}";
    assert_eq!(check(src), 5);
    assert!(
        ir_of(src).contains("call void @free("),
        "expected free() for the overwritten array buffer"
    );
}

#[test]
fn int_returning_function_frees_its_array_locals() {
    // v2: a function returning a non-array frees its array locals before `return`.
    let src = "\
fn sum3(n: Int) -> Int {
  let xs: IntArray = [n, n + 1, n + 2];
  return xs[0] + xs[1] + xs[2];
}
fn main() -> Int {
  return sum3(10);
}";
    assert_eq!(check(src), 33);
    assert!(
        ir_of(src).contains("call void @free("),
        "expected free() before the int return"
    );
}

#[test]
fn array_return_transfers_ownership_frees_other_locals() {
    // v3: a function returning an array transfers that buffer to the caller (no
    // copy) while freeing its OTHER array locals; the caller frees the
    // transferred buffer at its own scope exit. Correct result + no UAF/double
    // free (the differential check would catch either).
    let src = "\
fn build(n: Int) -> IntArray {
  let scratch: IntArray = [99, 99, 99];
  let result: IntArray = [n, n + 1];
  return result;
}
fn main() -> Int {
  let a: IntArray = build(5);
  return a[0] + a[1];
}";
    assert_eq!(check(src), 11);
    // `scratch` is freed inside build; the result is freed in main.
    assert!(ir_of(src).contains("call void @free("));
}

#[test]
fn nested_array_returning_calls() {
    // Chained array-returning calls: each result's ownership transfers cleanly.
    let src = "\
fn wrap(n: Int) -> IntArray { return [n, n * 2]; }
fn pick(n: Int) -> IntArray {
  let w: IntArray = wrap(n);
  return w;
}
fn main() -> Int {
  let a: IntArray = pick(6);
  return a[0] + a[1];
}";
    assert_eq!(check(src), 18);
}

#[test]
fn push_grow_frees_outgrown_buffer() {
    // v4: building an array by repeated in-place push doubles the buffer; each
    // outgrown buffer is freed (the local uniquely owns it). Correct + free emitted.
    let src = "\
import std.core;
fn main() -> Int {
  let mut xs: IntArray = int_array_empty();
  let mut i: Int = 0;
  while i < 50 {
    xs = int_array_push(xs, i);
    i = i + 1;
  }
  return xs[49] + xs[0];
}";
    assert_eq!(check(src), 49);
    assert!(ir_of(src).contains("call void @free("));
}

#[test]
fn fall_through_unit_function_frees_array_local() {
    // v4: a function that falls through (implicit return) frees its array locals.
    let src = "\
fn touch(n: Int) {
  let xs: IntArray = [n, n + 1];
  let s: Int = xs[0] + xs[1];
}
fn main() -> Int {
  touch(5);
  return 42;
}";
    assert_eq!(check(src), 42);
    assert!(ir_of(src).contains("call void @free("));
}

#[test]
fn string_locals_are_dropped() {
    // Strings are now value-managed: heap strings (concat) are freed at scope
    // exit via the generated @__drop_Str. Correct result + a free is emitted.
    let src = "\
import std.core;
fn main() -> Int {
  let a: String = string_concat(\"foo\", \"bar\");
  let b: String = string_concat(a, \"!\");
  return string_len(b);
}";
    assert_eq!(check(src), 7);
    assert!(ir_of(src).contains("call void @free("));
}

#[test]
fn string_array_managed_no_corruption() {
    // A string array's element strings are cloned in / dropped — building one and
    // iterating it must stay correct with no double-free/UAF (the differential
    // check + the no-crash run are the gate).
    let src = "\
import std.core;
fn main() -> Int {
  let mut parts: StringArray = string_array_empty();
  parts = string_array_push(parts, string_concat(\"a\", \"b\"));
  parts = string_array_push(parts, \"cde\");
  let mut total: Int = 0;
  for p in parts {
    total = total + string_len(p);
  }
  return total;
}";
    assert_eq!(check(src), 5);
}

#[test]
fn tuple_with_string_managed() {
    let src = "\
import std.core;
fn main() -> Int {
  let t: (Int, String) = (3, string_concat(\"xy\", \"z\"));
  return t.0 + string_len(t.1);
}";
    assert_eq!(check(src), 6);
    assert!(ir_of(src).contains("call void @free("));
}

#[test]
fn enum_string_payload_dropped_each_iteration() {
    // Each iteration boxes a heap String into an enum, matches it (binding clones an
    // independent copy), then both the enum and the binding go out of scope and are
    // dropped. No leak / double-free → differential result stays correct.
    let src = "\
import std.core;
enum Msg { Text(String), Empty }
fn main() -> Int {
  let mut total: Int = 0;
  let mut i: Int = 0;
  while i < 50 {
    let m: Msg = Msg.Text(string_concat(\"ab\", \"c\"));
    match m {
      Msg.Text(s) -> { total = total + string_len(s); }
      Msg.Empty -> {}
    }
    i = i + 1;
  }
  return total;
}";
    assert_eq!(check(src), 150);
    assert!(ir_of(src).contains("call void @free("));
}

#[test]
fn enum_struct_payload_box_freed() {
    // A struct payload is heap-BOXED in the enum (p1). Dropping the enum must drop
    // the boxed struct's managed field (its String) AND free the box itself.
    let src = "\
import std.core;
struct Tagged { name: String, n: Int }
enum Wrap { One(Tagged), Zero }
fn main() -> Int {
  let mut total: Int = 0;
  let mut i: Int = 0;
  while i < 20 {
    let w: Wrap = Wrap.One(Tagged { name: string_concat(\"x\", \"yz\"), n: i });
    match w {
      Wrap.One(t) -> { total = total + string_len(t.name) + t.n; }
      Wrap.Zero -> {}
    }
    i = i + 1;
  }
  return total;
}";
    // sum over i in 0..20 of (3 + i) = 60 + 190 = 250.
    assert_eq!(check(src), 250);
    assert!(ir_of(src).contains("call void @free("));
}

#[test]
fn closure_capturing_string_dropped_each_iteration() {
    // Each iteration captures a heap String into a closure env (cloned in), calls
    // it, then both `s` and the closure go out of scope. The closure's drop-thunk
    // drops the captured copy + frees the env; `s` frees its own. No double-free.
    let src = "\
import std.core;
fn main() -> Int {
  let mut total: Int = 0;
  let mut i: Int = 0;
  while i < 50 {
    let s: String = string_concat(\"ab\", \"c\");
    let f = || string_len(s);
    total = total + f();
    i = i + 1;
  }
  return total;
}";
    assert_eq!(check(src), 150);
    assert!(ir_of(src).contains("call void @free("));
}

#[test]
fn closure_capturing_string_escapes_then_dropped() {
    // A closure that captures a String is RETURNED (its env outlives the maker's
    // `s`). The caller drops the closure at scope exit → env String + env freed.
    let src = "\
import std.core;
fn make() -> fn() -> Int {
  let s: String = string_concat(\"hello\", \"!\");
  return || string_len(s);
}
fn main() -> Int {
  let f: fn() -> Int = make();
  return f();
}";
    assert_eq!(check(src), 6);
    assert!(ir_of(src).contains("call void @free("));
}

#[test]
fn loop_array_assigned_outward_stays_correct() {
    // The body-local array is deep-copied into the outer var on assignment, then
    // the body-local is freed — the outer copy must remain valid.
    let src = "\
fn main() -> Int {
  let mut keep: IntArray = [0];
  let mut i: Int = 0;
  while i < 5 {
    let tmp: IntArray = [i, i + 100];
    keep = tmp;
    i = i + 1;
  }
  return keep[0] + keep[1];
}";
    // last iteration i=4 → keep = [4, 104] → 108.
    assert_eq!(check(src), 108);
}
