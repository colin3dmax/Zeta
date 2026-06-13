// Hot reload — slice 2: the long-running service driver (init / step / render).
//
// `ServiceDriver` is the testable core the `zeta serve` CLI drives: it holds the
// live state and the swappable runtime. These tests prove the service-level
// guarantees:
//   1. tick advances state; render displays it (via the program's `render`).
//   2. reloading new SOURCE mid-stream changes behaviour while the accumulated
//      state is preserved.
//   3. a BAD reload (compile error in the new source) is rejected — the old code
//      and the state keep running, so a broken edit can't crash the service.

use zeta::runtime::{ServiceDriver, Value};

const COUNTER_V1: &str = "\
import std.core;
struct Counter { count: Int }
fn init() -> Counter { return Counter { count: 0 }; }
fn step(c: Counter, n: Int) -> Counter { return Counter { count: c.count + n }; }
fn render(c: Counter) -> String { return string_concat(\"count=\", int_to_string(c.count)); }
";

// Hot-swapped revision: scoring rule changes to n*10; everything else identical.
const COUNTER_V2: &str = "\
import std.core;
struct Counter { count: Int }
fn init() -> Counter { return Counter { count: 0 }; }
fn step(c: Counter, n: Int) -> Counter { return Counter { count: c.count + n * 10 }; }
fn render(c: Counter) -> String { return string_concat(\"count=\", int_to_string(c.count)); }
";

// Parses, but `nope` is undefined → rejected at resolve/typecheck inside reload.
const COUNTER_BROKEN: &str = "\
import std.core;
struct Counter { count: Int }
fn init() -> Counter { return Counter { count: 0 }; }
fn step(c: Counter, n: Int) -> Counter { return Counter { count: c.count + nope }; }
fn render(c: Counter) -> String { return string_concat(\"count=\", int_to_string(c.count)); }
";

#[test]
fn driver_ticks_and_renders() {
    let mut svc = ServiceDriver::start(COUNTER_V1).expect("service should start");
    svc.tick(Value::Int(3)).unwrap();
    svc.tick(Value::Int(5)).unwrap(); // Counter { count: 8 }
    assert_eq!(svc.render().unwrap(), "count=8");
}

#[test]
fn reload_changes_rule_but_preserves_state() {
    let mut svc = ServiceDriver::start(COUNTER_V1).unwrap();
    svc.tick(Value::Int(3)).unwrap();
    svc.tick(Value::Int(5)).unwrap();
    assert_eq!(svc.render().unwrap(), "count=8");

    // Hot-swap to the ×10 rule. The accumulated count (8) is untouched.
    svc.try_reload(COUNTER_V2).expect("valid reload should apply");

    svc.tick(Value::Int(2)).unwrap(); // 8 + 2*10 = 28
    assert_eq!(
        svc.render().unwrap(),
        "count=28",
        "carried state 8 + new ×10 rule"
    );
}

#[test]
fn bad_reload_is_rejected_and_service_survives() {
    let mut svc = ServiceDriver::start(COUNTER_V1).unwrap();
    svc.tick(Value::Int(3)).unwrap();
    svc.tick(Value::Int(5)).unwrap(); // count = 8
    assert_eq!(svc.render().unwrap(), "count=8");

    // A broken edit must be rejected with diagnostics, NOT applied or crashed.
    let outcome = svc.try_reload(COUNTER_BROKEN);
    assert!(
        outcome.is_err(),
        "compile error in new source should be rejected"
    );

    // The service keeps running the OLD code, with the state intact: the next
    // tick uses the original (additive) rule on the preserved count 8.
    svc.tick(Value::Int(2)).unwrap(); // 8 + 2 = 10 (old rule), NOT 28
    assert_eq!(
        svc.render().unwrap(),
        "count=10",
        "rejected reload left old code + state running"
    );
}

#[test]
fn render_falls_back_to_display_without_render_fn() {
    // No `render` fn → ServiceDriver falls back to the value's Display.
    const NO_RENDER: &str = "\
fn init() -> Int { return 0; }
fn step(s: Int, n: Int) -> Int { return s + n; }
";
    let mut svc = ServiceDriver::start(NO_RENDER).unwrap();
    svc.tick(Value::Int(7)).unwrap();
    assert_eq!(svc.render().unwrap(), "7");
}
