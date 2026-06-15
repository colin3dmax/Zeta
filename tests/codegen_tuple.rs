// Phase 2b: Tuple native codegen — differential against the interpreter.
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_tuple
//
// Tuples lower to LLVM anonymous structs (`{T0, T1, ...}`) with insert/extract
// for literals and `.N` access. `main` returns Int so the i64 JIT harness
// applies; the native JIT result must equal the Stage0 interpreter (run_mir).
#![cfg(feature = "llvm")]

use zeta::runtime::Value;

fn check(source: &str) -> i64 {
    let program = zeta::lower_source(source).expect("source should lower");
    let oracle = match zeta::runtime::run_mir(&program).expect("interpreter should run") {
        Value::Int(n) => n,
        other => panic!("expected Int from interpreter, got {other:?}"),
    };
    let native = zeta::codegen::jit_run_i64(&program, &[], "main").expect("native JIT should run");
    assert_eq!(
        native, oracle,
        "native/interpreter divergence\n--- source ---\n{source}\n--- native={native} oracle={oracle} ---"
    );
    oracle
}

#[test]
fn tuple_literal_and_index() {
    let src = "\
fn main() -> Int {
  let t = (10, 20, 30);
  return t.0 + t.1 + t.2;
}";
    assert_eq!(check(src), 60);
}

#[test]
fn tuple_heterogeneous() {
    let src = "\
fn main() -> Int {
  let t = (7, true);
  if t.1 { return t.0; }
  return 0;
}";
    assert_eq!(check(src), 7);
}

#[test]
fn tuple_nested_index() {
    let src = "\
fn main() -> Int {
  let t = (1, (2, 3));
  return t.0 + t.1.0 + t.1.1;
}";
    assert_eq!(check(src), 6);
}

#[test]
fn tuple_with_float_field() {
    let src = "\
fn main() -> Int {
  let t = (3, 1.5);
  let s: Float = t.1 + 2.5;
  if s > 3.9 { return t.0; }
  return 0;
}";
    assert_eq!(check(src), 3);
}

#[test]
fn tuple_rebind() {
    let src = "\
fn main() -> Int {
  let a = (4, 5);
  let b = a;
  return b.0 * b.1;
}";
    assert_eq!(check(src), 20);
}

#[test]
fn tuple_param_and_return() {
    // Tuple crosses a function boundary via type annotations on param and return.
    let src = "\
fn swap(p: (Int, Int)) -> (Int, Int) { return (p.1, p.0); }
fn main() -> Int {
  let t = swap((3, 8));
  return t.0 * 10 + t.1;
}";
    assert_eq!(check(src), 83);
}

#[test]
fn tuple_nested_param() {
    let src = "\
fn f(p: (Int, (Int, Int))) -> Int { return p.0 + p.1.0 + p.1.1; }
fn main() -> Int { return f((1, (2, 3))); }";
    assert_eq!(check(src), 6);
}

#[test]
fn tuple_in_loop() {
    // Rebuild a tuple each iteration and read its fields back.
    let src = "\
fn main() -> Int {
  let mut acc: Int = 0;
  let mut i: Int = 0;
  while i < 4 {
    let t = (i, i + 1);
    acc = acc + t.0 + t.1;
    i = i + 1;
  }
  return acc;
}";
    assert_eq!(check(src), 16);
}
