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
