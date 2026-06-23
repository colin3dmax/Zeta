// Phase 3a: Closure language layer — exercised through the Stage0 interpreter.
//
//   cargo test --test closure
//
// Lambdas `|x: Int| body` are first-class values that capture their free
// variables by value at creation time. A local of function type `fn(Int)->Int`
// is called indirectly. The MIR interpreter (`run_mir`) is the semantic oracle;
// native codegen (closure conversion) is a later slice.

use zeta::runtime::{run_mir, Value};

fn run(source: &str) -> Value {
    let program = zeta::lower_source(source).expect("source should lower");
    run_mir(&program).expect("interpreter should run")
}

fn check_err(source: &str) -> Vec<String> {
    match zeta::check_source(source) {
        Ok(_) => panic!("expected a compile error, source compiled cleanly"),
        Err(diagnostics) => diagnostics.into_iter().map(|d| d.code.to_string()).collect(),
    }
}

#[test]
fn lambda_no_capture_called() {
    let src = "\
fn main() -> Int {
  let add = |a: Int, b: Int| a + b;
  return add(3, 4);
}";
    assert_eq!(run(src), Value::Int(7));
}

#[test]
fn lambda_param_types_inferred_from_annotation() {
    // `|x|` (no param type) infers `x: Int` from the `fn(Int) -> Int` binding.
    let src = "\
fn main() -> Int {
  let k: Int = 10;
  let f: fn(Int) -> Int = |x| x + k;
  return f(5);
}";
    assert_eq!(run(src), Value::Int(15));
}

#[test]
fn lambda_inference_multi_param() {
    let src = "\
fn main() -> Int {
  let add: fn(Int, Int) -> Int = |a, b| a + b;
  return add(3, 4);
}";
    assert_eq!(run(src), Value::Int(7));
}

#[test]
fn lambda_inference_mixed_with_explicit() {
    // A partially-annotated list still fills only the empty ones.
    let src = "\
fn main() -> Int {
  let g: fn(Int, Int) -> Int = |a: Int, b| a * 10 + b;
  return g(4, 2);
}";
    assert_eq!(run(src), Value::Int(42));
}

#[test]
fn lambda_captures_local_by_value() {
    // The closure captures `n` at creation; mutating `n` afterwards does not
    // change what the closure observes (capture-by-value snapshot).
    let src = "\
fn main() -> Int {
  let mut n: Int = 10;
  let addn = |x: Int| x + n;
  n = 100;
  return addn(5);
}";
    assert_eq!(run(src), Value::Int(15));
}

#[test]
fn higher_order_param() {
    // A closure passed into a function and applied there.
    let src = "\
fn apply(f: fn(Int) -> Int, x: Int) -> Int { return f(x); }
fn main() -> Int {
  let double = |v: Int| v * 2;
  return apply(double, 21);
}";
    assert_eq!(run(src), Value::Int(42));
}

#[test]
fn closure_returned_from_function() {
    // A function that builds and returns a closure capturing its param.
    let src = "\
fn adder(n: Int) -> fn(Int) -> Int { return |x: Int| x + n; }
fn main() -> Int {
  let add7 = adder(7);
  return add7(35);
}";
    assert_eq!(run(src), Value::Int(42));
}

#[test]
fn zero_param_lambda() {
    let src = "\
fn main() -> Int {
  let answer = || 42;
  return answer();
}";
    assert_eq!(run(src), Value::Int(42));
}

#[test]
fn closure_over_float() {
    let src = "\
fn main() -> Int {
  let k: Float = 1.5;
  let scale = |x: Float| x * k;
  let r: Float = scale(4.0);
  if r > 5.9 { if r < 6.1 { return 1; } }
  return 0;
}";
    assert_eq!(run(src), Value::Int(1));
}

#[test]
fn lambda_body_unknown_name_rejected() {
    let src = "\
fn main() -> Int {
  let f = |x: Int| x + missing;
  return f(1);
}";
    assert!(check_err(src).iter().any(|c| c == "RESOLVE_UNKNOWN_NAME"));
}

#[test]
fn closure_call_arg_type_mismatch_rejected() {
    let src = "\
fn main() -> Int {
  let f = |x: Int| x + 1;
  return f(true);
}";
    assert!(check_err(src).iter().any(|c| c == "TYPE_CALL_ARGUMENT"));
}

#[test]
fn ast_dump_shows_lambda() {
    let dump = zeta::dump_ast("fn main() -> Int { let f = |x: Int| x + 1; return f(1); }")
        .expect("dump should succeed");
    assert!(dump.contains("Lambda"), "ast dump missing Lambda:\n{dump}");
}
