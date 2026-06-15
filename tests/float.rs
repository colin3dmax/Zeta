// Phase 1: Float (f64 scalar) — Stage0 language pipeline.
//
// Floats flow through lexer → parser → typecheck → MIR → runtime, mirroring Int
// but with f64. Arithmetic (+ - * /) and comparison work on Float; modulo /
// bitwise stay Int-only; Int and Float never mix implicitly. These tests check
// parse/dump self-consistency and run_mir results.

fn run(src: &str) -> String {
    zeta::run_source(src).expect("should run").to_string()
}

fn check_err(src: &str) -> Vec<String> {
    match zeta::run_source(src) {
        Ok(_) => panic!("expected a type error, but it ran"),
        Err(diags) => diags.iter().map(|d| d.code.to_string()).collect(),
    }
}

#[test]
fn float_literal_and_add() {
    assert_eq!(run("fn main() -> Float { return 1.5 + 2.5; }"), "4.0");
}

#[test]
fn float_arithmetic() {
    assert_eq!(
        run("fn main() -> Float { let x: Float = 3.0; let y: Float = 2.0; return x * y - x / y; }"),
        "4.5",
    );
}

#[test]
fn float_neg() {
    assert_eq!(run("fn main() -> Float { let x: Float = 2.5; return 0.0 - x; }"), "-2.5");
}

#[test]
fn float_unary_neg() {
    assert_eq!(run("fn main() -> Float { return -2.5; }"), "-2.5");
}

#[test]
fn float_comparison() {
    let src = "\
fn main() -> Int {
  let a: Float = 1.5;
  let b: Float = 2.0;
  if a < b { if b > a { return 1; } }
  return 0;
}";
    assert_eq!(run(src), "1");
}

#[test]
fn float_integral_displays_with_point() {
    // A Float that happens to be integral still prints as 4.0, not 4.
    assert_eq!(run("fn main() -> Float { return 2.0 + 2.0; }"), "4.0");
}

#[test]
fn float_param_and_return() {
    let src = "\
fn scale(x: Float, k: Float) -> Float { return x * k; }
fn main() -> Float { return scale(1.5, 4.0); }";
    assert_eq!(run(src), "6.0");
}

#[test]
fn ast_dump_shows_float() {
    let ast = zeta::dump_ast("fn main() -> Float { return 1.5; }").expect("ast-dump");
    assert!(ast.contains("Float 1.5"), "ast-dump missing Float node:\n{ast}");
}

#[test]
fn mir_dump_shows_float() {
    let mir = zeta::dump_mir("fn main() -> Float { return 1.5; }").expect("mir-dump");
    assert!(mir.contains("const Float 1.5"), "mir-dump missing Float:\n{mir}");
}

#[test]
fn int_and_float_do_not_mix() {
    // Adding Int + Float is a type error (no implicit coercion).
    let codes = check_err("fn main() -> Float { return 1 + 2.5; }");
    assert!(
        codes.iter().any(|c| c == "TYPE_BINARY_OPERAND"),
        "expected TYPE_BINARY_OPERAND, got {codes:?}",
    );
}

#[test]
fn float_modulo_rejected() {
    let codes = check_err("fn main() -> Float { return 5.0 % 2.0; }");
    assert!(
        codes.iter().any(|c| c == "TYPE_BINARY_OPERAND"),
        "expected TYPE_BINARY_OPERAND for float %, got {codes:?}",
    );
}
