// Hot reload — slice 3: `reloadable` as an explicit language construct.
//
// `reloadable fn` marks a coarse-grained hot-swap boundary (docs/compiler/
// hot-reload-design.md §3). These tests cover:
//   1. the modifier parses onto the AST Function flag (and is false otherwise);
//   2. it flows through to MIR;
//   3. the runtime ENFORCES the discipline: a `reloadable` function may change
//      across a reload, but changing a non-`reloadable` function is rejected —
//      so the perf-critical static/inlinable code can't be silently swapped.

use zeta::ast::Item;
use zeta::runtime::{ServiceDriver, Value};

#[test]
fn reloadable_modifier_parses_onto_the_flag() {
    let module = zeta::parse_source(
        "reloadable fn step() -> Int { return 0; } fn plain() -> Int { return 1; }",
    )
    .expect("should parse");

    let mut seen = 0;
    for item in &module.items {
        if let Item::Function(function) = item {
            seen += 1;
            match function.name.as_str() {
                "step" => assert!(function.reloadable, "`reloadable fn step` should set the flag"),
                "plain" => assert!(!function.reloadable, "plain `fn` should not be reloadable"),
                other => panic!("unexpected function {other}"),
            }
        }
    }
    assert_eq!(seen, 2);
}

#[test]
fn reloadable_is_only_consumed_before_fn() {
    // `reloadable` is contextual: usable as an ordinary identifier elsewhere.
    let module = zeta::parse_source("fn f() -> Int { let reloadable: Int = 5; return reloadable; }")
        .expect("`reloadable` should still be a valid identifier");
    assert_eq!(module.items.len(), 1);
}

#[test]
fn reloadable_flag_flows_to_mir() {
    let program = zeta::lower_source(
        "reloadable fn step() -> Int { return 0; } fn plain() -> Int { return 1; }",
    )
    .expect("should lower");
    let step = program.functions.iter().find(|f| f.name == "step").unwrap();
    let plain = program.functions.iter().find(|f| f.name == "plain").unwrap();
    assert!(step.reloadable);
    assert!(!plain.reloadable);
}

// A service whose `step` is reloadable but whose helper `bump` is NOT.
const HELPER_V1: &str = "\
import std.core;
struct S { v: Int }
fn init() -> S { return S { v: 0 }; }
fn bump(x: Int) -> Int { return x + 1; }
reloadable fn step(s: S, n: Int) -> S { return S { v: bump(s.v) + n }; }
fn render(s: S) -> String { return int_to_string(s.v); }
";

// Changes ONLY the reloadable `step` (×10 on input). Allowed.
const HELPER_STEP_CHANGED: &str = "\
import std.core;
struct S { v: Int }
fn init() -> S { return S { v: 0 }; }
fn bump(x: Int) -> Int { return x + 1; }
reloadable fn step(s: S, n: Int) -> S { return S { v: bump(s.v) + n * 10 }; }
fn render(s: S) -> String { return int_to_string(s.v); }
";

// Changes the NON-reloadable helper `bump`. Must be rejected.
const HELPER_BUMP_CHANGED: &str = "\
import std.core;
struct S { v: Int }
fn init() -> S { return S { v: 0 }; }
fn bump(x: Int) -> Int { return x + 100; }
reloadable fn step(s: S, n: Int) -> S { return S { v: bump(s.v) + n }; }
fn render(s: S) -> String { return int_to_string(s.v); }
";

#[test]
fn changing_a_reloadable_function_is_allowed() {
    let mut svc = ServiceDriver::start(HELPER_V1).unwrap();
    svc.tick(Value::Int(3)).unwrap(); // v = bump(0)+3 = 4
    svc.try_reload(HELPER_STEP_CHANGED)
        .expect("changing the reloadable step should be allowed");
    svc.tick(Value::Int(2)).unwrap(); // v = bump(4)+2*10 = 5+20 = 25
    assert_eq!(svc.render().unwrap(), "25");
}

#[test]
fn changing_a_non_reloadable_function_is_rejected() {
    let mut svc = ServiceDriver::start(HELPER_V1).unwrap();
    svc.tick(Value::Int(3)).unwrap(); // v = 4

    let outcome = svc.try_reload(HELPER_BUMP_CHANGED);
    let diagnostics = outcome.expect_err("changing non-reloadable `bump` must be rejected");
    assert_eq!(diagnostics[0].code, "HOT_RELOAD_NON_RELOADABLE");
    assert!(
        diagnostics[0].message.contains("bump"),
        "the rejection should name the offending function `bump`, got: {}",
        diagnostics[0].message
    );

    // The service kept the OLD code: next tick uses the original bump (+1) and
    // additive step on the preserved state (v=4): v = bump(4)+5 = 5+5 = 10.
    svc.tick(Value::Int(5)).unwrap();
    assert_eq!(svc.render().unwrap(), "10");
}
