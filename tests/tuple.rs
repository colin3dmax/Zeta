// Phase 2a: Tuple language layer — differential against the Stage0 interpreter.
//
//   cargo test --test tuple
//
// Tuples are fixed-arity heterogeneous aggregates: `(a, b)` literals and `.0`/`.1`
// index access. Type inference works inside a function body; tuple *type
// annotations* (params/returns) are a deferred increment, so these tests keep
// tuples local. The interpreter (`run_mir`) is the semantic oracle.

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
fn tuple_literal_and_index() {
    let src = "\
fn main() -> Int {
  let t = (10, 20, 30);
  return t.0 + t.1 + t.2;
}";
    assert_eq!(run(src), Value::Int(60));
}

#[test]
fn tuple_heterogeneous_fields() {
    // Fields keep their own static types: t.0 is Int, t.1 is Bool.
    let src = "\
fn main() -> Int {
  let t = (7, true);
  if t.1 { return t.0; }
  return 0;
}";
    assert_eq!(run(src), Value::Int(7));
}

#[test]
fn tuple_nested() {
    let src = "\
fn main() -> Int {
  let t = (1, (2, 3));
  return t.0 + t.1.0 + t.1.1;
}";
    assert_eq!(run(src), Value::Int(6));
}

#[test]
fn tuple_of_float() {
    let src = "\
fn main() -> Int {
  let t = (1.5, 2.5);
  let s: Float = t.0 + t.1;
  if s > 3.9 { return 1; }
  return 0;
}";
    assert_eq!(run(src), Value::Int(1));
}

#[test]
fn tuple_rebind_field() {
    // A tuple bound to another name reads back the same components.
    let src = "\
fn main() -> Int {
  let a = (4, 5);
  let b = a;
  return b.0 * b.1;
}";
    assert_eq!(run(src), Value::Int(20));
}

#[test]
fn parenthesized_expr_is_not_a_tuple() {
    // `(expr)` without a comma is still plain grouping, not a 1-tuple.
    let src = "\
fn main() -> Int {
  let x = (3 + 4) * 2;
  return x;
}";
    assert_eq!(run(src), Value::Int(14));
}

#[test]
fn tuple_index_out_of_range_rejected() {
    let src = "\
fn main() -> Int {
  let t = (1, 2);
  return t.2;
}";
    assert!(check_err(src).iter().any(|c| c == "TYPE_TUPLE_INDEX"));
}

#[test]
fn tuple_param_annotation() {
    // A tuple type annotation `(Int, Int)` lets a tuple cross a function boundary.
    let src = "\
fn sum(p: (Int, Int)) -> Int { return p.0 + p.1; }
fn main() -> Int {
  let t = (8, 9);
  return sum(t);
}";
    assert_eq!(run(src), Value::Int(17));
}

#[test]
fn tuple_return_annotation() {
    let src = "\
fn pair(a: Int, b: Int) -> (Int, Int) { return (a, b); }
fn main() -> Int {
  let t: (Int, Int) = pair(3, 4);
  return t.0 * t.1;
}";
    assert_eq!(run(src), Value::Int(12));
}

#[test]
fn tuple_nested_annotation() {
    let src = "\
fn f(p: (Int, (Int, Int))) -> Int { return p.0 + p.1.0 + p.1.1; }
fn main() -> Int {
  return f((1, (2, 3)));
}";
    assert_eq!(run(src), Value::Int(6));
}

#[test]
fn grouped_type_annotation_is_not_a_tuple() {
    // `(Int)` with no comma is just grouping and behaves like a plain `Int`.
    let src = "\
fn main() -> Int {
  let x: (Int) = 5;
  return x;
}";
    assert_eq!(run(src), Value::Int(5));
}

#[test]
fn tuple_annotation_mismatch_rejected() {
    // Returning a 2-tuple where a 3-tuple is declared is a type error.
    let src = "\
fn f() -> (Int, Int, Int) { return (1, 2); }
fn main() -> Int { return 0; }";
    assert!(!check_err(src).is_empty());
}

#[test]
fn ast_dump_shows_tuple() {
    let dump = zeta::dump_ast("fn main() -> Int { let t = (1, 2); return t.0; }")
        .expect("dump should succeed");
    assert!(dump.contains("Tuple"), "ast dump missing Tuple:\n{dump}");
}

#[test]
fn mir_dump_shows_tuple() {
    let dump = zeta::dump_mir("fn main() -> Int { let t = (1, 2); return t.0; }")
        .expect("dump should succeed");
    assert!(dump.contains("tuple ("), "mir dump missing tuple:\n{dump}");
}
