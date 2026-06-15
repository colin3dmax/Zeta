// Phase 3b: Closure native codegen — differential against the interpreter.
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_closure
//
// Closures lower via closure conversion: each lambda is lifted to a top-level
// LLVM function taking a heap environment pointer, and the closure value is
// `{ fn_ptr, env_ptr }`. Indirect calls load and invoke through it. `main`
// returns Int (i64 JIT harness); the native result must equal `run_mir`.
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
fn lambda_no_capture() {
    let src = "\
fn main() -> Int {
  let add = |a: Int, b: Int| a + b;
  return add(3, 4);
}";
    assert_eq!(check(src), 7);
}

#[test]
fn lambda_captures_local_by_value() {
    // Capture `n` at creation; mutating it afterwards must not change the result.
    let src = "\
fn main() -> Int {
  let mut n: Int = 10;
  let addn = |x: Int| x + n;
  n = 100;
  return addn(5);
}";
    assert_eq!(check(src), 15);
}

#[test]
fn higher_order_param() {
    let src = "\
fn apply(f: fn(Int) -> Int, x: Int) -> Int { return f(x); }
fn main() -> Int {
  let double = |v: Int| v * 2;
  return apply(double, 21);
}";
    assert_eq!(check(src), 42);
}

#[test]
fn closure_returned_from_function() {
    let src = "\
fn adder(n: Int) -> fn(Int) -> Int { return |x: Int| x + n; }
fn main() -> Int {
  let add7 = adder(7);
  return add7(35);
}";
    assert_eq!(check(src), 42);
}

#[test]
fn zero_param_lambda() {
    let src = "\
fn main() -> Int {
  let answer = || 42;
  return answer();
}";
    assert_eq!(check(src), 42);
}

#[test]
fn closure_multi_capture() {
    let src = "\
fn main() -> Int {
  let a: Int = 3;
  let b: Int = 4;
  let c: Int = 5;
  let f = |x: Int| x + a + b + c;
  return f(10);
}";
    assert_eq!(check(src), 22);
}

#[test]
fn closure_over_float_reduced_to_int() {
    let src = "\
fn main() -> Int {
  let k: Float = 1.5;
  let scale = |x: Float| x * k;
  let r: Float = scale(4.0);
  if r > 5.9 { if r < 6.1 { return 1; } }
  return 0;
}";
    assert_eq!(check(src), 1);
}

#[test]
fn closure_called_twice() {
    let src = "\
fn main() -> Int {
  let base: Int = 100;
  let f = |x: Int| x + base;
  return f(1) + f(2);
}";
    assert_eq!(check(src), 203);
}
