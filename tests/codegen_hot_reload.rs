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

use zeta::ast::Item;
use zeta::codegen::{NativeArrayService, NativeService, NativeStructService};
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

// Non-scalar (IntArray) state across a native hot-swap. The state buffer is on
// the C heap (libc malloc), so it survives the engine swap; only the code is
// remapped.
const ARR_V1: &str = "\
fn init() -> IntArray { return [0, 0, 0]; }
reloadable fn step(s: IntArray, n: Int) -> IntArray {
  let mut t: IntArray = s;
  t[0] = t[0] + n;
  return t;
}";
const ARR_V2: &str = "\
fn init() -> IntArray { return [0, 0, 0]; }
reloadable fn step(s: IntArray, n: Int) -> IntArray {
  let mut t: IntArray = s;
  t[0] = t[0] + n * 10;
  return t;
}";

#[test]
fn native_array_state_survives_hot_swap() {
    let mut svc = NativeArrayService::start(&program(ARR_V1), &[]).expect("array service");
    assert_eq!(svc.len(), 3);

    svc.tick(3); // [3,0,0]
    svc.tick(5); // [8,0,0]
    assert_eq!(svc.get(0), 8);

    // Hot-swap to ×10 rule; the heap-backed array state (8,0,0) survives.
    svc.reload(&program(ARR_V2), &[]).expect("array reload");
    svc.tick(2); // [8 + 2*10, 0, 0] = [28,0,0]

    assert_eq!(svc.get(0), 28, "array element carried across swap + new native rule");
    assert_eq!(svc.len(), 3);
    assert_eq!(svc.get(1), 0);
}

// Struct-typed state across a native hot-swap. The state blob is Rust-owned
// (8-byte-aligned), so it survives the engine swap; pointer wrappers bridge the
// per-struct ABI.
const STRUCT_V1: &str = "\
struct Counter { value: Int, ticks: Int }
fn init() -> Counter { return Counter { value: 0, ticks: 0 }; }
reloadable fn step(s: Counter, n: Int) -> Counter {
  return Counter { value: s.value + n, ticks: s.ticks + 1 };
}";
const STRUCT_V2: &str = "\
struct Counter { value: Int, ticks: Int }
fn init() -> Counter { return Counter { value: 0, ticks: 0 }; }
reloadable fn step(s: Counter, n: Int) -> Counter {
  return Counter { value: s.value + n * 10, ticks: s.ticks + 1 };
}";

fn structs_of(src: &str) -> Vec<zeta::ast::StructDecl> {
    zeta::parse_source(src)
        .unwrap()
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(decl) => Some(decl.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn native_struct_state_survives_hot_swap() {
    let mut svc =
        NativeStructService::start(&program(STRUCT_V1), &structs_of(STRUCT_V1)).expect("struct service");

    svc.tick(3); // value 3, ticks 1
    svc.tick(5); // value 8, ticks 2
    assert_eq!(svc.field_i64(0), 8, "value field");
    assert_eq!(svc.field_i64(1), 2, "ticks field");

    // Hot-swap to the ×10 rule; the struct state (value 8, ticks 2) survives.
    svc.reload(&program(STRUCT_V2), &structs_of(STRUCT_V2)).expect("struct reload");
    svc.tick(2); // value 8 + 2*10 = 28, ticks 3

    assert_eq!(svc.field_i64(0), 28, "value carried across swap + new native rule");
    assert_eq!(svc.field_i64(1), 3, "ticks carried + incremented");
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
