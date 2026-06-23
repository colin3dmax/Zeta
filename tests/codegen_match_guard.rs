// Match guards: `pat if <cond> -> ..`. The arm is taken only when the pattern
// matches AND the guard is true; otherwise control falls through to the next arm.
// A guarded arm never contributes to exhaustiveness (its guard may fail), so a
// plain catch-all is still required. These tests confirm the interpreter (the
// differential oracle) and the native backend agree on every guard shape.
#![cfg(feature = "llvm")]

use zeta::runtime::Value;

/// Lower `source`, run it through the interpreter and the native JIT, assert they
/// agree, and return main's Int.
fn check(source: &str) -> i64 {
    let program = zeta::lower_source(source).expect("lower");
    let oracle = match zeta::runtime::run_mir(&program).expect("interpreter") {
        Value::Int(n) => n,
        other => panic!("expected Int, got {other:?}"),
    };
    let native = zeta::codegen::jit_run_i64(&program, &[], "main").expect("native JIT");
    assert_eq!(native, oracle, "native/interpreter divergence");
    oracle
}

/// Same, but expect lowering to fail (e.g. a non-exhaustive match).
fn expect_lower_error(source: &str) -> Vec<String> {
    match zeta::lower_source(source) {
        Ok(_) => panic!("expected lowering to fail"),
        Err(diags) => diags.into_iter().map(|d| d.code.to_string()).collect(),
    }
}

#[test]
fn int_guard_first_match_wins() {
    // sign(n): the first arm whose guard passes wins; the `_` catches zero.
    let src = "\
fn sign(n: Int) -> Int {
  match n {
    x if x > 0 -> { return 1; },
    x if x < 0 -> { return 0 - 1; },
    _ -> { return 0; },
  }
}
fn main() -> Int {
  return sign(7) * 100 + (sign(0 - 4) + 1) * 10 + sign(0);
}";
    // sign(7)=1 -> 100; sign(-4)=-1 -> (-1+1)*10=0; sign(0)=0 -> 0. Total 100.
    assert_eq!(check(src), 100);
}

#[test]
fn enum_payload_guard_falls_through() {
    // The guard reads the variant's Int payload; a false guard falls through to
    // the next (unguarded) Some arm.
    let src = "\
enum Opt { Some(Int), None }
fn describe(o: Opt) -> Int {
  match o {
    Opt.Some(n) if n > 10 -> { return 2; },
    Opt.Some(n) -> { return 1; },
    Opt.None -> { return 0; },
  }
}
fn main() -> Int {
  return describe(Opt.Some(42)) * 100 + describe(Opt.Some(3)) * 10 + describe(Opt.None);
}";
    // big=2 ->200; small=1 ->10; none=0. Total 210.
    assert_eq!(check(src), 210);
}

#[test]
fn multiple_guards_same_pattern() {
    // Several guarded arms with the same binding pattern, distinguished only by
    // their guards — the classic use case a `switch` cannot express.
    let src = "\
fn bucket(n: Int) -> Int {
  match n {
    x if x < 10 -> { return 0; },
    x if x < 100 -> { return 1; },
    x if x < 1000 -> { return 2; },
    _ -> { return 3; },
  }
}
fn main() -> Int {
  return bucket(5) + bucket(50) * 10 + bucket(500) * 100 + bucket(5000) * 1000;
}";
    // 0 + 1*10 + 2*100 + 3*1000 = 3210.
    assert_eq!(check(src), 3210);
}

#[test]
fn string_scrutinee_guard() {
    // A guard on a String scrutinee: the chain falls back from a guarded literal
    // arm to a plain one. `len` lets the guard distinguish without extra state.
    let src = "\
import std.core;
fn route(path: String) -> Int {
  match path {
    \"/\" -> { return 0; },
    p if string_len(p) > 4 -> { return 2; },
    _ -> { return 1; },
  }
}
fn main() -> Int {
  return route(\"/\") * 100 + route(\"/longpath\") * 10 + route(\"/ab\");
}";
    // "/"=0 ->0; "/longpath" len 9>4 =2 ->20; "/ab" len 3 =1. Total 21.
    assert_eq!(check(src), 21);
}

#[test]
fn string_payload_binding_guard_clone_path() {
    // The guard reads a String payload binding (exercises the clone-on-bind path),
    // and a false guard falls through to the next arm — confirming the cloned
    // binding doesn't corrupt the enum's own copy (no double-free / no garbage).
    let src = "\
import std.core;
enum Msg { Text(String), Empty }
fn weight(m: Msg) -> Int {
  match m {
    Msg.Text(s) if string_len(s) > 3 -> { return string_len(s); },
    Msg.Text(s) -> { return 0; },
    Msg.Empty -> { return 0 - 1; },
  }
}
fn main() -> Int {
  return weight(Msg.Text(\"hello\")) * 100 + weight(Msg.Text(\"hi\")) * 10 + (weight(Msg.Empty) + 1);
}";
    // "hello" len 5>3 ->5*100=500; "hi" len 2 ->0; Empty ->-1 -> (-1+1)=0. Total 500.
    assert_eq!(check(src), 500);
}

#[test]
fn guarded_catch_all_is_not_exhaustive() {
    // The `Some` variant is covered only by a GUARDED arm, which can fail — so
    // the enum match isn't exhaustive and the typechecker must reject it.
    let src = "\
enum Opt { Some(Int), None }
fn f(o: Opt) -> Int {
  match o {
    Opt.Some(n) if n > 0 -> { return 1; },
    Opt.None -> { return 0; },
  }
}
fn main() -> Int { return f(Opt.None); }";
    let codes = expect_lower_error(src);
    assert!(
        codes.iter().any(|c| c == "TYPE_MATCH_NON_EXHAUSTIVE"),
        "expected non-exhaustive diagnostic, got {codes:?}"
    );
}

#[test]
fn guard_must_be_bool() {
    // A non-Bool guard expression is a type error.
    let src = "\
fn f(n: Int) -> Int {
  match n {
    x if x + 1 -> { return 1; },
    _ -> { return 0; },
  }
}
fn main() -> Int { return f(1); }";
    let codes = expect_lower_error(src);
    assert!(
        codes.iter().any(|c| c == "TYPE_MATCH_GUARD_NOT_BOOL"),
        "expected guard-not-Bool diagnostic, got {codes:?}"
    );
}
