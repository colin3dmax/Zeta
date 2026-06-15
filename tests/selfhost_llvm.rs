// Stage2 (slice 1) differential gate: the Zeta-side LLVM IR emitter.
//
// `testdata/selfhost/arena_frontend.zeta` now exposes `compile(src, "llvm")`,
// which walks the MIR arena and emits textual LLVM IR for the scalar subset
// (Int/Bool as i64; let/assign/return; arithmetic/bitwise/comparison/unary/
// logical; if/else/while/break/continue; user calls incl. recursion). This gate
// runs that emitter inside the Stage0 interpreter, compiles the emitted IR with
// clang, links a tiny C driver that prints `z_main()`, runs it, and asserts the
// printed result equals the Stage0 interpreter oracle (`run_mir`) for the same
// program. So: the Zeta-emitted native code reproduces the interpreter exactly.
//
// Needs the LLVM toolchain, so it is gated behind the `llvm` cargo feature and
// run alongside the other codegen_* gates:
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test selfhost_llvm
#![cfg(feature = "llvm")]

use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

const FRONTEND_SOURCE: &str = include_str!("../testdata/selfhost/arena_frontend.zeta");

fn source_file(path: &str, source: &str) -> zeta::module_graph::SourceFile {
    zeta::module_graph::SourceFile {
        path: path.to_string(),
        source: source.to_string(),
    }
}

/// Escape a Zeta source so it can be embedded inside a double-quoted Zeta
/// string literal in the caller app.
fn zeta_string_literal(source: &str) -> String {
    let mut out = String::from("\"");
    for ch in source.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// Run `compile(program_source, "llvm")` inside the Zeta interpreter and return
/// the textual LLVM IR it produces.
fn emit_ir(program_source: &str) -> String {
    let caller = format!(
        r#"
module selfhost.caller;
import selfhost.arena_frontend;

fn main() -> String {{
  let source: String = {literal};
  return selfhost.arena_frontend.compile(source, "llvm");
}}
"#,
        literal = zeta_string_literal(program_source),
    );
    let value = zeta::module_graph::run_sources(&[
        source_file("testdata/selfhost/arena_frontend.zeta", FRONTEND_SOURCE),
        source_file("testdata/selfhost/caller.zeta", &caller),
    ])
    .expect("llvm emit caller should run");
    value.to_string()
}

/// The Stage0 interpreter oracle: lower + run the program, expecting an Int.
fn oracle(program_source: &str) -> i64 {
    let program = zeta::lower_source(program_source).expect("oracle should lower");
    match zeta::runtime::run_mir(&program).expect("oracle should run") {
        zeta::runtime::Value::Int(n) => n,
        other => panic!("oracle produced non-Int: {other:?}"),
    }
}

fn clang_path() -> String {
    match std::env::var("LLVM_SYS_221_PREFIX") {
        Ok(prefix) => format!("{prefix}/bin/clang"),
        Err(_) => "clang".to_string(),
    }
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Compile the emitted IR with clang, link a C driver that prints `z_main()`,
/// run it, and return the printed i64.
fn run_native(ir: &str) -> i64 {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("zeta_llvm_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).expect("temp dir");

    let ll = dir.join("entry.ll");
    std::fs::File::create(&ll)
        .and_then(|mut f| f.write_all(ir.as_bytes()))
        .expect("write .ll");

    let driver = dir.join("driver.c");
    std::fs::File::create(&driver)
        .and_then(|mut f| {
            f.write_all(b"#include <stdio.h>\nlong z_main(void);\nint main(void){ printf(\"%ld\\n\", z_main()); return 0; }\n")
        })
        .expect("write driver.c");

    let exe = dir.join("prog");
    let compile = Command::new(clang_path())
        .arg(&ll)
        .arg(&driver)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("invoke clang");
    assert!(
        compile.status.success(),
        "clang failed to compile emitted IR:\n--- stderr ---\n{}\n--- IR ---\n{ir}",
        String::from_utf8_lossy(&compile.stderr),
    );

    let run = Command::new(&exe).output().expect("run native exe");
    assert!(run.status.success(), "native exe exited with failure");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let result = stdout
        .trim()
        .parse::<i64>()
        .unwrap_or_else(|_| panic!("native printed non-integer: {stdout:?}"));

    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// Assert the Zeta-emitted native code matches the interpreter oracle.
fn check(program_source: &str) -> i64 {
    let want = oracle(program_source);
    let ir = emit_ir(program_source);
    let got = run_native(&ir);
    assert_eq!(
        got, want,
        "native/interpreter divergence\n--- source ---\n{program_source}\n--- IR ---\n{ir}\n--- native={got} oracle={want} ---"
    );
    want
}

#[test]
fn let_and_arithmetic() {
    assert_eq!(check("fn main() -> Int { let x: Int = 40; return x + 2; }"), 42);
}

#[test]
fn arithmetic_and_bitwise_precedence() {
    let src = "fn main() -> Int { return (7 * 6) - (10 / 2) % 3 + (12 & 6) | 1; }";
    check(src);
}

#[test]
fn comparison_into_if() {
    let src = "fn main() -> Int { let a: Int = 5; if a > 3 { return 1; } return 0; }";
    assert_eq!(check(src), 1);
}

#[test]
fn logical_not_and_comparison() {
    let src = "fn main() -> Int { let b: Bool = 3 > 5; if !b { return 7; } return 0; }";
    assert_eq!(check(src), 7);
}

#[test]
fn if_else_chain_via_calls() {
    let src = "\
fn classify(n: Int) -> Int {
  if n < 0 { return 0 - 1; }
  if n == 0 { return 0; }
  return 1;
}
fn main() -> Int {
  return classify(0 - 7) * 100 + classify(0) * 10 + classify(9);
}";
    check(src);
}

#[test]
fn while_loop_sum() {
    let src = "\
fn main() -> Int {
  let mut s: Int = 0;
  let mut i: Int = 0;
  while i < 10 { s = s + i; i = i + 1; }
  return s;
}";
    assert_eq!(check(src), 45);
}

#[test]
fn recursive_fib() {
    let src = "\
fn fib(n: Int) -> Int {
  if n < 2 { return n; }
  return fib(n - 1) + fib(n - 2);
}
fn main() -> Int { return fib(20); }";
    assert_eq!(check(src), 6765);
}

#[test]
fn nested_loops() {
    let src = "\
fn main() -> Int {
  let mut acc: Int = 0;
  let mut i: Int = 0;
  while i < 5 {
    let mut j: Int = 0;
    while j < 5 { acc = acc + 1; j = j + 1; }
    i = i + 1;
  }
  return acc;
}";
    assert_eq!(check(src), 25);
}

#[test]
fn break_and_continue() {
    let src = "\
fn main() -> Int {
  let mut s: Int = 0;
  let mut i: Int = 0;
  while i < 100 {
    i = i + 1;
    if i > 10 { break; }
    if i % 2 == 0 { continue; }
    s = s + i;
  }
  return s;
}";
    // odds 1..=9: 1+3+5+7+9
    assert_eq!(check(src), 25);
}

#[test]
fn logical_and_or() {
    let src = "\
fn main() -> Int {
  let a: Int = 5;
  let mut r: Int = 0;
  if a > 0 && a < 10 { r = 1; }
  if a > 100 || a == 5 { r = r + 10; }
  return r;
}";
    assert_eq!(check(src), 11);
}

// --- slice 2: struct codegen (literal / field / param / return / nested) ---

#[test]
fn struct_literal_and_field_read() {
    let src = "\
struct Point { x: Int, y: Int }
fn main() -> Int {
  let p: Point = Point { x: 3, y: 4 };
  return p.x + p.y;
}";
    assert_eq!(check(src), 7);
}

#[test]
fn struct_field_mutation() {
    let src = "\
struct Point { x: Int, y: Int }
fn main() -> Int {
  let mut p: Point = Point { x: 3, y: 4 };
  p.x = 10;
  return p.x * 100 + p.y;
}";
    assert_eq!(check(src), 1004);
}

#[test]
fn struct_param_by_value() {
    let src = "\
struct Point { x: Int, y: Int }
fn area(p: Point) -> Int { return p.x * p.y; }
fn main() -> Int {
  let p: Point = Point { x: 6, y: 7 };
  return area(p);
}";
    assert_eq!(check(src), 42);
}

#[test]
fn struct_returned_from_function() {
    let src = "\
struct Point { x: Int, y: Int }
fn mk(a: Int, b: Int) -> Point { return Point { x: a, y: b }; }
fn main() -> Int {
  let p: Point = mk(8, 9);
  return p.x * 10 + p.y;
}";
    assert_eq!(check(src), 89);
}

#[test]
fn nested_struct_read_and_write() {
    let src = "\
struct Point { x: Int, y: Int }
struct Line { a: Point, b: Point }
fn main() -> Int {
  let mut l: Line = Line { a: Point { x: 1, y: 2 }, b: Point { x: 3, y: 4 } };
  l.a.x = 100;
  return l.a.x + l.b.y;
}";
    assert_eq!(check(src), 104);
}

#[test]
fn struct_assignment_is_value_copy() {
    let src = "\
struct Point { x: Int, y: Int }
fn main() -> Int {
  let p: Point = Point { x: 1, y: 2 };
  let mut q: Point = p;
  q.x = 99;
  return p.x * 10 + q.x;
}";
    // p stays {1,2}; q becomes {99,2}
    assert_eq!(check(src), 109);
}

// --- slice 3: IntArray codegen (literal / index / len / push / value copy) ---

#[test]
fn array_literal_index_and_len() {
    let src = "\
fn main() -> Int {
  let xs: IntArray = [10, 20, 30];
  return xs.len * 1000 + xs[0] + xs[2];
}";
    assert_eq!(check(src), 3040);
}

#[test]
fn array_index_write() {
    let src = "\
fn main() -> Int {
  let mut xs: IntArray = [1, 2, 3];
  xs[1] = 99;
  return xs[0] + xs[1] + xs[2];
}";
    assert_eq!(check(src), 103);
}

#[test]
fn array_empty_push_and_sum() {
    let src = "\
import std.core;
fn main() -> Int {
  let mut xs: IntArray = int_array_empty();
  let mut i: Int = 0;
  while i < 5 { xs = int_array_push(xs, i * i); i = i + 1; }
  let mut s: Int = 0;
  let mut j: Int = 0;
  while j < xs.len { s = s + xs[j]; j = j + 1; }
  return s * 100 + xs.len;
}";
    // squares 0+1+4+9+16 = 30, len 5
    assert_eq!(check(src), 3005);
}

#[test]
fn array_assignment_is_value_copy() {
    let src = "\
fn main() -> Int {
  let xs: IntArray = [1, 2, 3];
  let mut ys: IntArray = xs;
  ys[0] = 99;
  return xs[0] * 10 + ys[0];
}";
    // xs[0] stays 1; ys[0] becomes 99
    assert_eq!(check(src), 109);
}

#[test]
fn array_param_read() {
    let src = "\
fn sum2(a: IntArray) -> Int { return a[0] + a[1]; }
fn main() -> Int {
  let xs: IntArray = [7, 8];
  return sum2(xs);
}";
    assert_eq!(check(src), 15);
}

#[test]
fn array_copy_independent_of_later_push() {
    let src = "\
import std.core;
fn main() -> Int {
  let mut xs: IntArray = int_array_empty();
  xs = int_array_push(xs, 1);
  xs = int_array_push(xs, 2);
  let ys: IntArray = xs;
  xs = int_array_push(xs, 3);
  return ys.len * 10 + xs.len;
}";
    // ys keeps [1,2] (len 2); xs grows to [1,2,3] (len 3)
    assert_eq!(check(src), 23);
}

// --- slice 4: string codegen (literal / builtins / equality) ---

#[test]
fn string_literal_and_len() {
    let src = "\
import std.core;
fn main() -> Int {
  let s: String = \"hello\";
  return string_len(s);
}";
    assert_eq!(check(src), 5);
}

#[test]
fn string_byte_at_value() {
    let src = "\
import std.core;
fn main() -> Int { return string_byte_at(\"hello\", 0); }";
    // 'h'
    assert_eq!(check(src), 104);
}

#[test]
fn string_concat_len_and_byte() {
    let src = "\
import std.core;
fn main() -> Int {
  let s: String = string_concat(\"ab\", \"cde\");
  return string_len(s) * 100 + string_byte_at(s, 2);
}";
    // "abcde" len 5, byte[2]='c'(99)
    assert_eq!(check(src), 599);
}

#[test]
fn string_equality_and_inequality() {
    let src = "\
import std.core;
fn main() -> Int {
  if \"abc\" == \"abc\" {
    if \"abc\" != \"abd\" {
      return 1;
    }
  }
  return 0;
}";
    assert_eq!(check(src), 1);
}

#[test]
fn string_byte_slice_value() {
    let src = "\
import std.core;
fn main() -> Int {
  let s: String = string_byte_slice(\"hello\", 1, 3);
  return string_len(s) * 100 + string_byte_at(s, 0);
}";
    // "ell" len 3, byte[0]='e'(101)
    assert_eq!(check(src), 401);
}

#[test]
fn int_to_string_roundtrip() {
    let src = "\
import std.core;
fn main() -> Int {
  let s: String = int_to_string(42);
  return string_len(s) * 100 + string_byte_at(s, 0);
}";
    // "42" len 2, byte[0]='4'(52)
    assert_eq!(check(src), 252);
}

#[test]
fn ascii_predicates() {
    let src = "\
import std.core;
fn main() -> Int {
  let mut r: Int = 0;
  if ascii_is_digit(53) { r = r + 1; }
  if ascii_is_alpha(65) { r = r + 10; }
  if ascii_is_alnum(122) { r = r + 100; }
  if ascii_is_whitespace(32) { r = r + 1000; }
  if !ascii_is_digit(65) { r = r + 10000; }
  return r;
}";
    assert_eq!(check(src), 11111);
}

#[test]
fn string_param_and_loop() {
    let src = "\
import std.core;
fn count_l(s: String) -> Int {
  let mut n: Int = 0;
  let mut i: Int = 0;
  while i < string_len(s) {
    if string_byte_at(s, i) == 108 { n = n + 1; }
    i = i + 1;
  }
  return n;
}
fn main() -> Int { return count_l(\"hello\"); }";
    // 'l' appears twice
    assert_eq!(check(src), 2);
}

#[test]
fn string_returned_from_function() {
    let src = "\
import std.core;
fn greet() -> String { return string_concat(\"hi\", \"!\"); }
fn main() -> Int {
  let s: String = greet();
  return string_len(s);
}";
    assert_eq!(check(src), 3);
}

#[test]
fn string_literal_newline_escape() {
    // `\n` in a string literal must decode to a real newline (byte 10), matching
    // the Stage0 lexer — not the literal letter 'n'.
    let src = "\
import std.core;
fn main() -> Int {
  let s: String = \"a\\nb\";
  return string_len(s) * 100 + string_byte_at(s, 1);
}";
    // "a\nb": len 3, byte[1] = '\n' (10)
    assert_eq!(check(src), 310);
}

// --- slice 5: enum + match codegen (switch on tag / value) ---

#[test]
fn enum_payloadless_dispatch() {
    let src = "\
enum Color { Red, Green, Blue }
fn code(c: Color) -> Int {
  match c {
    Color.Red -> { return 1; }
    Color.Green -> { return 2; }
    Color.Blue -> { return 3; }
  }
  return 0;
}
fn main() -> Int {
  return code(Color.Red) * 100 + code(Color.Green) * 10 + code(Color.Blue);
}";
    assert_eq!(check(src), 123);
}

#[test]
fn enum_int_payload_bind() {
    let src = "\
enum OptionInt { Some(Int), None }
fn unwrap_or(v: OptionInt, f: Int) -> Int {
  match v {
    OptionInt.Some(n) -> { return n; }
    OptionInt.None -> { return f; }
  }
  return 0;
}
fn main() -> Int {
  let a: OptionInt = OptionInt.Some(40);
  let b: OptionInt = OptionInt.None;
  return unwrap_or(a, 99) + unwrap_or(b, 99);
}";
    // 40 + 99
    assert_eq!(check(src), 139);
}

#[test]
fn enum_wildcard_default() {
    let src = "\
enum Dir { North, East, South, West }
fn main() -> Int {
  let d: Dir = Dir.South;
  match d {
    Dir.North -> { return 1; }
    _ -> { return 99; }
  }
  return 0;
}";
    assert_eq!(check(src), 99);
}

#[test]
fn match_over_int_value() {
    let src = "\
fn classify(n: Int) -> Int {
  match n {
    0 -> { return 100; }
    1 -> { return 200; }
    _ -> { return 300; }
  }
  return 0;
}
fn main() -> Int { return classify(0) + classify(1) + classify(5); }";
    // 100 + 200 + 300
    assert_eq!(check(src), 600);
}

#[test]
fn match_over_bool_value() {
    let src = "\
fn pick(b: Bool) -> Int {
  match b {
    true -> { return 11; }
    false -> { return 22; }
  }
  return 0;
}
fn main() -> Int { return pick(true) * 100 + pick(false); }";
    assert_eq!(check(src), 1122);
}

#[test]
fn match_name_binding_default() {
    let src = "\
fn f(n: Int) -> Int {
  match n {
    7 -> { return 0; }
    other -> { return other * 2; }
  }
  return 0;
}
fn main() -> Int { return f(7) + f(21); }";
    // 0 + 42
    assert_eq!(check(src), 42);
}

#[test]
fn enum_string_payload() {
    let src = "\
import std.core;
enum Msg { Text(String), Empty }
fn describe(m: Msg) -> Int {
  match m {
    Msg.Text(s) -> { return string_len(s) * 100 + string_byte_at(s, 0); }
    Msg.Empty -> { return 0 - 1; }
  }
  return 0;
}
fn main() -> Int {
  let a: Msg = Msg.Text(\"hi\");
  let b: Msg = Msg.Empty;
  return describe(a) * 10 + describe(b) + 1;
}";
    // describe(a): len 2 * 100 + 'h'(104) = 304; describe(b) = -1
    // 304*10 + (-1) + 1 = 3040
    assert_eq!(check(src), 3040);
}

#[test]
fn match_falls_through_to_end() {
    let src = "\
enum Sign { Neg, Zero, Pos }
fn main() -> Int {
  let s: Sign = Sign.Pos;
  let mut score: Int = 0;
  match s {
    Sign.Neg -> { score = 1; }
    Sign.Zero -> { score = 2; }
    Sign.Pos -> { score = 3; }
  }
  return score * 10;
}";
    assert_eq!(check(src), 30);
}

#[test]
fn match_in_loop() {
    let src = "\
enum Step { Inc, Dec }
fn main() -> Int {
  let mut acc: Int = 0;
  let mut i: Int = 0;
  while i < 5 {
    let s: Step = Step.Inc;
    match s {
      Step.Inc -> { acc = acc + 2; }
      Step.Dec -> { acc = acc - 1; }
    }
    i = i + 1;
  }
  return acc;
}";
    assert_eq!(check(src), 10);
}

// --- slice 6: for-loop codegen (ForRange / ForIn / ForC) ---

#[test]
fn for_range_sum() {
    let src = "\
fn main() -> Int {
  let mut s: Int = 0;
  for i in 0..10 { s = s + i; }
  return s;
}";
    assert_eq!(check(src), 45);
}

#[test]
fn for_range_break_continue() {
    let src = "\
fn main() -> Int {
  let mut s: Int = 0;
  for i in 0..20 {
    if i >= 10 { break; }
    if i % 2 == 0 { continue; }
    s = s + i;
  }
  return s;
}";
    // odds below 10: 1+3+5+7+9
    assert_eq!(check(src), 25);
}

#[test]
fn for_c_sum() {
    let src = "\
fn main() -> Int {
  let mut s: Int = 0;
  for (let mut i: Int = 0; i < 10; i = i + 1) { s = s + i; }
  return s;
}";
    assert_eq!(check(src), 45);
}

#[test]
fn for_range_nested() {
    let src = "\
fn main() -> Int {
  let mut acc: Int = 0;
  for i in 0..3 {
    for j in 0..3 { acc = acc + 1; }
  }
  return acc;
}";
    assert_eq!(check(src), 9);
}

#[test]
fn for_in_array_sum() {
    let src = "\
import std.core;
fn main() -> Int {
  let mut xs: IntArray = int_array_empty();
  xs = int_array_push(xs, 3);
  xs = int_array_push(xs, 4);
  xs = int_array_push(xs, 5);
  let mut s: Int = 0;
  for x in xs { s = s + x; }
  return s;
}";
    assert_eq!(check(src), 12);
}

#[test]
fn for_in_with_continue() {
    let src = "\
import std.core;
fn main() -> Int {
  let mut xs: IntArray = int_array_empty();
  for k in 0..6 { xs = int_array_push(xs, k); }
  let mut s: Int = 0;
  for x in xs {
    if x % 2 == 0 { continue; }
    s = s + x;
  }
  return s;
}";
    // odds 1+3+5
    assert_eq!(check(src), 9);
}

// --- slice 7: StringArray / BoolArray (generalized element type) ---

#[test]
fn string_array_literal_and_index() {
    let src = "\
import std.core;
fn main() -> Int {
  let xs: StringArray = [\"ab\", \"cde\"];
  return string_len(xs[0]) + string_len(xs[1]);
}";
    assert_eq!(check(src), 5);
}

#[test]
fn string_array_push_and_for_in() {
    let src = "\
import std.core;
fn main() -> Int {
  let mut xs: StringArray = string_array_empty();
  xs = string_array_push(xs, \"aa\");
  xs = string_array_push(xs, \"bbb\");
  let mut t: Int = 0;
  for s in xs { t = t + string_len(s); }
  return t;
}";
    assert_eq!(check(src), 5);
}

#[test]
fn string_array_index_write() {
    let src = "\
import std.core;
fn main() -> Int {
  let mut xs: StringArray = [\"x\", \"yy\"];
  xs[0] = \"zzz\";
  return string_len(xs[0]) * 10 + string_len(xs[1]);
}";
    assert_eq!(check(src), 32);
}

#[test]
fn bool_array_push_and_count() {
    let src = "\
import std.core;
fn main() -> Int {
  let mut xs: BoolArray = bool_array_empty();
  xs = bool_array_push(xs, true);
  xs = bool_array_push(xs, false);
  xs = bool_array_push(xs, true);
  let mut c: Int = 0;
  for b in xs {
    if b { c = c + 1; }
  }
  return c;
}";
    assert_eq!(check(src), 2);
}

#[test]
fn string_array_copy_independent() {
    let src = "\
import std.core;
fn main() -> Int {
  let mut xs: StringArray = string_array_empty();
  xs = string_array_push(xs, \"a\");
  xs = string_array_push(xs, \"b\");
  let ys: StringArray = xs;
  xs = string_array_push(xs, \"c\");
  return ys.len * 10 + xs.len;
}";
    // ys keeps 2, xs grows to 3
    assert_eq!(check(src), 23);
}

// --- slice 8: block scoping (same name, different type in disjoint branches) ---

#[test]
fn shadowed_local_different_types() {
    let src = "\
struct Foo { x: Int }
fn f(c: Bool) -> Int {
  if c {
    let v: Foo = Foo { x: 5 };
    return v.x;
  } else {
    let v: Int = 9;
    return v;
  }
}
fn main() -> Int { return f(true) + f(false); }";
    // f(true)=5, f(false)=9
    assert_eq!(check(src), 14);
}

#[test]
fn shadowed_local_array_then_scalar() {
    let src = "\
import std.core;
fn g(c: Bool) -> Int {
  if c {
    let v: IntArray = [3, 4, 5];
    return v.len;
  } else {
    let v: String = \"hello\";
    return string_len(v);
  }
}
fn main() -> Int { return g(true) * 10 + g(false); }";
    // g(true)=3, g(false)=5
    assert_eq!(check(src), 35);
}

// --- Float (P1) back-ported into the self-hosting emitter --------------------
// `double` constants + fadd/fsub/fmul/fdiv + fcmp; main returns Int (the driver
// reads z_main as i64), so floats are reduced to an Int via comparison.

#[test]
fn float_add_compare_emit() {
    let src = "\
fn main() -> Int {
  let x: Float = 1.5 + 2.5;
  if x > 3.9 { return 1; }
  return 0;
}";
    assert_eq!(check(src), 1);
}

#[test]
fn float_arithmetic_chain_emit() {
    let src = "\
fn main() -> Int {
  let x: Float = 3.0;
  let y: Float = 2.0;
  let r: Float = x * y - x / y;
  if r > 4.4 { if r < 4.6 { return 7; } }
  return 0;
}";
    assert_eq!(check(src), 7);
}

#[test]
fn float_neg_and_param_emit() {
    let src = "\
fn scale(x: Float, k: Float) -> Float { return x * k; }
fn main() -> Int {
  let r: Float = scale(1.5, 4.0);
  let n: Float = -r;
  if n < 0.0 { if r > 5.9 { if r < 6.1 { return 3; } } }
  return 0;
}";
    assert_eq!(check(src), 3);
}

#[test]
fn float_equality_emit() {
    let src = "\
fn main() -> Int {
  let a: Float = 1.0 + 1.0;
  if a == 2.0 { if a != 3.0 { return 1; } }
  return 0;
}";
    assert_eq!(check(src), 1);
}

#[test]
fn float_loop_accumulate_emit() {
    let src = "\
fn main() -> Int {
  let mut acc: Float = 0.0;
  let mut i: Int = 0;
  while i < 4 { acc = acc + 0.5; i = i + 1; }
  if acc > 1.9 { if acc < 2.1 { return 1; } }
  return 0;
}";
    assert_eq!(check(src), 1);
}

/// Capstone probe: emit LLVM IR for the ENTIRE self-hosting frontend via the
/// Zeta-side codegen (`compile(frontend, "llvm")`) and assert clang compiles it
/// to an object. Proves the Zeta emitter covers every construct the frontend
/// uses (no unsupported expression falls back to garbage IR).
#[test]
#[ignore]
fn frontend_emits_compilable_ir() {
    let ir = emit_ir(FRONTEND_SOURCE);
    assert!(
        ir.len() > 50_000,
        "emitted IR is suspiciously small ({} bytes) — emission likely failed",
        ir.len()
    );
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("zeta_frontend_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let ll = dir.join("frontend.ll");
    std::fs::write(&ll, &ir).expect("write frontend.ll");
    let obj = dir.join("frontend.o");
    let out = Command::new(clang_path())
        .arg("-c")
        .arg(&ll)
        .arg("-o")
        .arg(&obj)
        .output()
        .expect("invoke clang");
    assert!(
        out.status.success(),
        "clang failed to compile the frontend IR:\n{}",
        String::from_utf8_lossy(&out.stderr),
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Run-level capstone: the ENTIRE frontend compiled by its own Zeta-side codegen
/// must RUN correctly. We append a `main` that calls `compile(<small program>,
/// "mir-dump")` and reduces the dump to an Int checksum, then assert the native
/// build (Zeta-emitted IR via clang) produces the same checksum as the Stage0
/// interpreter — i.e. the frontend, compiled by itself to native code, behaves
/// byte-for-byte like the reference.
#[test]
#[ignore]
fn frontend_runs_native_matches_oracle() {
    let appended = "\n\
fn z_checksum(s: String) -> Int {\n\
\x20 let mut h: Int = 0;\n\
\x20 let mut i: Int = 0;\n\
\x20 while i < string_len(s) {\n\
\x20   h = h + string_byte_at(s, i) * (i + 1);\n\
\x20   i = i + 1;\n\
\x20 }\n\
\x20 return h;\n\
}\n\
fn main() -> Int {\n\
\x20 let src: String = \"fn main() -> Int { let a: Int = 5; let b: Int = 7; return a + b; }\";\n\
\x20 let dump: String = compile(src, \"mir-dump\");\n\
\x20 return z_checksum(dump);\n\
}\n";
    let mut combined = String::from(FRONTEND_SOURCE);
    combined.push_str(appended);
    check(&combined);
}

/// Measurement (not an assertion): time each phase and compare a native
/// frontend `compile(prog,"mir-dump")` call to the interpreter doing the same,
/// to find the real bottleneck before optimizing.
#[test]
#[ignore]
fn bench_native_run() {
    use std::time::Instant;
    let mut prog = String::new();
    for i in 0..50 {
        prog.push_str(&format!(
            "fn f{i}(x: Int) -> Int {{ let a: Int = x + {i}; if a > 5 {{ return a * 2; }} return a; }}\n"
        ));
    }
    prog.push_str("fn main() -> Int { return f0(1); }\n");

    let t = Instant::now();
    let ir = emit_ir(FRONTEND_SOURCE);
    println!("emit_ir: {:.1}s, ir={} KB", t.elapsed().as_secs_f64(), ir.len() / 1024);

    let k: usize = 200;
    let dir = std::env::temp_dir().join(format!("zeta_bench_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("f.ll"), &ir).unwrap();
    let driver = format!(
        "#include <stdio.h>\n#include <string.h>\n#include <stdlib.h>\nstruct ZStr {{ long len; char* ptr; }};\nstruct ZStr z_compile(struct ZStr, struct ZStr);\nint main(int c, char**v){{ int K=atoi(v[1]); const char* p={prog:?}; const char* m=\"mir-dump\"; struct ZStr s={{(long)strlen(p),(char*)p}}; struct ZStr mo={{(long)strlen(m),(char*)m}}; long acc=0; for(int i=0;i<K;i++){{ struct ZStr r=z_compile(s,mo); acc+=r.len; }} printf(\"%ld\\n\",acc); return 0; }}\n",
        prog = prog,
    );
    std::fs::write(dir.join("d.c"), driver).unwrap();
    let tc = Instant::now();
    let exe = dir.join("b");
    let cc = Command::new(clang_path())
        .arg("-O2")
        .arg(dir.join("f.ll"))
        .arg(dir.join("d.c"))
        .arg("-o")
        .arg(&exe)
        .output()
        .unwrap();
    assert!(cc.status.success(), "{}", String::from_utf8_lossy(&cc.stderr));
    println!("clang -O2: {:.1}s", tc.elapsed().as_secs_f64());
    let tr = Instant::now();
    let run = Command::new(&exe).arg(k.to_string()).output().unwrap();
    assert!(run.status.success());
    let nt = tr.elapsed().as_secs_f64();
    println!("native: {} calls in {:.2}s = {:.3}ms/call", k, nt, nt * 1000.0 / k as f64);

    let combined = format!(
        "{FRONTEND_SOURCE}\nfn zz() -> Int {{ let d: String = compile({lit}, \"mir-dump\"); return string_len(d); }}\nfn main() -> Int {{ return zz(); }}\n",
        lit = zeta_string_literal(&prog),
    );
    let ti = Instant::now();
    let _ = zeta::module_graph::run_sources(&[source_file(
        "testdata/selfhost/arena_frontend.zeta",
        &combined,
    )])
    .unwrap();
    println!("interp: 1 call in {:.2}s", ti.elapsed().as_secs_f64());
    let _ = std::fs::remove_dir_all(&dir);
}
