// Native backend × hot reload (cargo feature `llvm`): a service whose `step`
// runs as optimized NATIVE code and is hot-swapped without losing state. This is
// where the two project lines converge — C-speed execution + state-preserving
// hot reload (docs/compiler/hot-reload-design.md §3.1).
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_hot_reload
//
// The interpreter `ServiceDriver` is the differential oracle: the native service
// must produce the same state sequence across the same ticks + reload.
#![cfg(feature = "llvm")]

use zeta::codegen::NativeService;
use zeta::runtime::{ServiceDriver, Value};

const V1: &str = "\
fn init() -> Int { return 0; }
reloadable fn step(state: Int, input: Int) -> Int { return state + input; }
fn render(s: Int) -> Int { return s; }";

// Hot-swapped: step now adds input*10.
const V2: &str = "\
fn init() -> Int { return 0; }
reloadable fn step(state: Int, input: Int) -> Int { return state + input * 10; }
fn render(s: Int) -> Int { return s; }";

fn program(src: &str) -> zeta::mir::Program {
    zeta::lower_source(src).unwrap()
}

fn oracle_state(value: Value) -> i64 {
    match value {
        Value::Int(n) => n,
        other => panic!("{other:?}"),
    }
}

#[test]
fn native_service_state_survives_native_hot_swap() {
    let mut svc = NativeService::start(&program(V1), &[]).expect("native service start");

    assert_eq!(svc.tick(3), 3); // 0 + 3
    assert_eq!(svc.tick(5), 8); // 3 + 5

    // Hot-swap to native V2; accumulated state (8) is preserved.
    svc.reload(&program(V2), &[]).expect("native reload");

    assert_eq!(svc.tick(2), 28); // 8 + 2*10  ← carried state + new native code
    assert_eq!(svc.state(), 28);
}

#[test]
fn native_matches_interpreter_service_across_reload() {
    // Drive the SAME tick/reload script on the native service and the
    // interpreter ServiceDriver; their state sequences must agree.
    let mut native = NativeService::start(&program(V1), &[]).unwrap();
    let mut interp = ServiceDriver::start(V1).unwrap();

    let inputs_before = [1, 2, 7];
    for &x in &inputs_before {
        let n = native.tick(x);
        let o = oracle_state(interp.tick(Value::Int(x)).unwrap());
        assert_eq!(n, o, "pre-reload tick {x}: native {n} vs interpreter {o}");
    }

    native.reload(&program(V2), &[]).unwrap();
    interp.try_reload(V2).unwrap();

    let inputs_after = [3, 4];
    for &x in &inputs_after {
        let n = native.tick(x);
        let o = oracle_state(interp.tick(Value::Int(x)).unwrap());
        assert_eq!(n, o, "post-reload tick {x}: native {n} vs interpreter {o}");
    }
}

#[test]
fn no_swap_control_differs() {
    // Without the swap the native sequence stays on V1's additive rule.
    let mut svc = NativeService::start(&program(V1), &[]).unwrap();
    for x in [1, 2, 3, 4] {
        svc.tick(x);
    }
    assert_eq!(svc.state(), 10); // 1+2+3+4, vs the swapped run's larger total
}
