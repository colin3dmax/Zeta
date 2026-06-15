// Phase 4b: Generics native codegen — monomorphization, differential vs interpreter.
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_generics
//
// Each call to a generic function is monomorphized on demand: a specialized
// LLVM function is generated for the concrete argument types (e.g. `id$Int`)
// and cached. `main` returns Int (i64 JIT harness); native must equal `run_mir`.
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
fn identity_int() {
    let src = "\
fn id<T>(x: T) -> T { return x; }
fn main() -> Int { return id(42); }";
    assert_eq!(check(src), 42);
}

#[test]
fn identity_two_instances() {
    // id is instantiated at Int and at Bool — two distinct specializations.
    let src = "\
fn id<T>(x: T) -> T { return x; }
fn main() -> Int {
  let b = id(true);
  if b { return id(7); }
  return 0;
}";
    assert_eq!(check(src), 7);
}

#[test]
fn generic_pair_and_first() {
    let src = "\
fn pair<A, B>(a: A, b: B) -> (A, B) { return (a, b); }
fn fst<A, B>(p: (A, B)) -> A { return p.0; }
fn main() -> Int {
  let t = pair(11, true);
  return fst(t);
}";
    assert_eq!(check(src), 11);
}

#[test]
fn generic_pick_first() {
    let src = "\
fn pick<A, B>(a: A, b: B) -> A { return a; }
fn main() -> Int { return pick(9, true); }";
    assert_eq!(check(src), 9);
}

#[test]
fn generic_flows_into_concrete() {
    let src = "\
fn id<T>(x: T) -> T { return x; }
fn inc(n: Int) -> Int { return n + 1; }
fn main() -> Int { return inc(id(41)); }";
    assert_eq!(check(src), 42);
}

#[test]
fn generic_result_in_arithmetic() {
    let src = "\
fn id<T>(x: T) -> T { return x; }
fn main() -> Int { return id(20) + id(22); }";
    assert_eq!(check(src), 42);
}

#[test]
fn generic_float_instance() {
    let src = "\
fn id<T>(x: T) -> T { return x; }
fn main() -> Int {
  let r: Float = id(3.5);
  if r > 3.4 { if r < 3.6 { return 1; } }
  return 0;
}";
    assert_eq!(check(src), 1);
}

#[test]
fn generic_instance_cached_across_calls() {
    // Two calls at the same type reuse one specialization (same result either way).
    let src = "\
fn id<T>(x: T) -> T { return x; }
fn main() -> Int { return id(40) + id(2); }";
    assert_eq!(check(src), 42);
}

#[test]
fn generic_transitive_specialization() {
    // A generic function calling another generic function.
    let src = "\
fn id<T>(x: T) -> T { return x; }
fn twice<T>(x: T) -> T { return id(x); }
fn main() -> Int { return twice(42); }";
    assert_eq!(check(src), 42);
}
