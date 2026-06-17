// Phase 1b: Float (f64) native codegen — differential against the interpreter.
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_float
//
// `main` returns Int (so the i64 JIT harness applies); Float values are exercised
// internally and reduced to an Int via comparison. The native JIT result must
// equal the Stage0 interpreter (run_mir).
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
fn float_add_compare() {
    let src = "\
fn main() -> Int {
  let x: Float = 1.5 + 2.5;
  if x > 3.9 { return 1; }
  return 0;
}";
    assert_eq!(check(src), 1);
}

#[test]
fn float_arithmetic_chain() {
    // (3.0 * 2.0 - 3.0 / 2.0) = 4.5; check via comparison.
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
fn float_neg_and_lt() {
    let src = "\
fn main() -> Int {
  let x: Float = 2.5;
  let n: Float = -x;
  if n < 0.0 { return 1; }
  return 0;
}";
    assert_eq!(check(src), 1);
}

#[test]
fn float_param_return_roundtrip() {
    // A Float-typed param/return path: scale then compare.
    let src = "\
fn scale(x: Float, k: Float) -> Float { return x * k; }
fn main() -> Int {
  let r: Float = scale(1.5, 4.0);
  if r > 5.9 { if r < 6.1 { return 3; } }
  return 0;
}";
    assert_eq!(check(src), 3);
}

#[test]
fn float_equality() {
    let src = "\
fn main() -> Int {
  let a: Float = 1.0 + 1.0;
  if a == 2.0 { if a != 3.0 { return 1; } }
  return 0;
}";
    assert_eq!(check(src), 1);
}

#[test]
fn float_local_mutation_in_loop() {
    // Accumulate a float in a while loop, then compare.
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

#[test]
fn float_array_native() {
    // FloatArray (P3 first slice): f64 elements, native must equal interpreter.
    let src = "\
import std.core;
fn main() -> Int {
  let mut xs: FloatArray = float_array_empty();
  xs = float_array_push(xs, 1.5);
  xs = float_array_push(xs, 2.5);
  let lit: FloatArray = [10.0, 20.0];
  let s: Float = xs[0] + xs[1] + lit[0] + lit[1];
  if s > 33.9 { if s < 34.1 { return xs.len + lit.len; } }
  return 0;
}";
    assert_eq!(check(src), 4);
}
