// Phase 4a: Generics language layer — exercised through the Stage0 interpreter.
//
//   cargo test --test generics
//
// Generic functions `fn id<T>(x: T) -> T` are parametrically polymorphic; type
// parameters are inferred from argument types at the call site. The interpreter
// is dynamically typed (Values carry their kind), so generic functions run
// directly without monomorphization — that is a native-only concern (4b). The
// MIR interpreter (`run_mir`) is the semantic oracle.

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
fn identity_int() {
    let src = "\
fn id<T>(x: T) -> T { return x; }
fn main() -> Int { return id(42); }";
    assert_eq!(run(src), Value::Int(42));
}

#[test]
fn identity_reused_at_two_types() {
    // The same generic function instantiated at Int and at Bool in one program.
    let src = "\
fn id<T>(x: T) -> T { return x; }
fn main() -> Int {
  let b = id(true);
  if b { return id(7); }
  return 0;
}";
    assert_eq!(run(src), Value::Int(7));
}

#[test]
fn generic_pair_and_first() {
    // `pair` builds a tuple; `fst` projects it. Two type params each.
    let src = "\
fn pair<A, B>(a: A, b: B) -> (A, B) { return (a, b); }
fn fst<A, B>(p: (A, B)) -> A { return p.0; }
fn main() -> Int {
  let t = pair(11, true);
  return fst(t);
}";
    assert_eq!(run(src), Value::Int(11));
}

#[test]
fn generic_const_picks_first() {
    let src = "\
fn pick<A, B>(a: A, b: B) -> A { return a; }
fn main() -> Int { return pick(9, true); }";
    assert_eq!(run(src), Value::Int(9));
}

#[test]
fn generic_passes_through_concrete_callee() {
    // A generic value flows into a concrete function once instantiated.
    let src = "\
fn id<T>(x: T) -> T { return x; }
fn inc(n: Int) -> Int { return n + 1; }
fn main() -> Int { return inc(id(41)); }";
    assert_eq!(run(src), Value::Int(42));
}

#[test]
fn generic_return_used_in_arithmetic() {
    // id(5) instantiates T=Int, so the result is usable as an Int.
    let src = "\
fn id<T>(x: T) -> T { return x; }
fn main() -> Int { return id(20) + id(22); }";
    assert_eq!(run(src), Value::Int(42));
}

#[test]
fn generic_body_arithmetic_on_type_param_rejected() {
    // `x + 1` requires a concrete numeric type; an unconstrained T does not
    // support it.
    let src = "\
fn bad<T>(x: T) -> T { return x + 1; }
fn main() -> Int { return 0; }";
    assert!(!check_err(src).is_empty());
}

#[test]
fn generic_arg_inconsistent_binding_rejected() {
    // `same<T>(a: T, b: T)` called with mixed types must fail to unify T.
    let src = "\
fn same<T>(a: T, b: T) -> T { return a; }
fn main() -> Int { return same(1, true); }";
    assert!(check_err(src).iter().any(|c| c == "TYPE_CALL_ARGUMENT"));
}

#[test]
fn ast_dump_unaffected_runs() {
    // Generic functions parse and lower cleanly end-to-end.
    let dump = zeta::dump_ast("fn id<T>(x: T) -> T { return x; } fn main() -> Int { return id(1); }")
        .expect("dump should succeed");
    assert!(dump.contains("Call callee=id"), "ast dump:\n{dump}");
}
