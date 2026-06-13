// Hot reload — slice 1: state-preserving runtime hot code swap.
//
// A `HotRuntime` lives across many `step` calls. The program STATE is a value
// threaded by the driver (this test) through `call(..)` and back; the swappable
// code lives in the runtime's function table. `hot_swap` atomically replaces
// that table with a newly lowered program — and the SAME accumulated state is
// then fed to the new code. See docs/compiler/hot-reload-design.md.
//
// These tests are the kernel proof: (1) state survives the swap, (2) the new
// function body takes effect afterwards, (3) a no-swap control yields a
// different sequence (so the swap is what changed behaviour).

use zeta::runtime::{HotRuntime, Value};

fn as_int(value: Value) -> i64 {
    match value {
        Value::Int(n) => n,
        other => panic!("expected Int, got {other:?}"),
    }
}

fn program(source: &str) -> zeta::mir::Program {
    zeta::lower_source(source).expect("service source should lower")
}

const V1: &str = "fn step(state: Int, input: Int) -> Int { return state + input; }";
// Hot-swapped revision: same signature, new body (doubles the input).
const V2: &str = "fn step(state: Int, input: Int) -> Int { return state + input * 2; }";

#[test]
fn state_survives_hot_swap_and_new_body_takes_effect() {
    let mut rt = HotRuntime::new(&program(V1));

    // Accumulate state with the original `step` (state + input).
    let mut state = Value::Int(0);
    state = rt.call("step", vec![state, Value::Int(1)]).unwrap(); // 0 + 1 = 1
    state = rt.call("step", vec![state, Value::Int(2)]).unwrap(); // 1 + 2 = 3
    assert_eq!(as_int(state.clone()), 3, "pre-swap accumulation");

    // Hot-swap to the new `step` (state + input*2). The accumulated state (3) is
    // untouched by the swap.
    rt.hot_swap(&program(V2));

    state = rt.call("step", vec![state, Value::Int(3)]).unwrap(); // 3 + 3*2 = 9
    state = rt.call("step", vec![state, Value::Int(4)]).unwrap(); // 9 + 4*2 = 17

    // 17 proves BOTH facts: the post-swap result built on the carried state 3
    // (not a fresh 0), and the new doubling body was used.
    assert_eq!(as_int(state), 17, "state carried across swap + new body applied");
}

#[test]
fn without_swap_the_sequence_differs() {
    // Control: identical inputs, but never swap — stays on V1 (state + input).
    let mut rt = HotRuntime::new(&program(V1));
    let mut state = Value::Int(0);
    for input in [1, 2, 3, 4] {
        state = rt.call("step", vec![state, Value::Int(input)]).unwrap();
    }
    // 0+1+2+3+4 = 10, vs 17 with the swap — so the swap is what changed behaviour.
    assert_eq!(as_int(state), 10);
}

// A realistic struct-shaped state, produced by the program's own `init()` and
// preserved across a swap that changes the scoring rule.
const WORLD_V1: &str = "\
struct World { score: Int, tick: Int }
fn init() -> World { return World { score: 0, tick: 0 }; }
fn step(w: World, pts: Int) -> World { return World { score: w.score + pts, tick: w.tick + 1 }; }
fn read_score(w: World) -> Int { return w.score; }
fn read_tick(w: World) -> Int { return w.tick; }
";
// Hot-swapped: scoring changes to pts*10; everything else identical.
const WORLD_V2: &str = "\
struct World { score: Int, tick: Int }
fn init() -> World { return World { score: 0, tick: 0 }; }
fn step(w: World, pts: Int) -> World { return World { score: w.score + pts * 10, tick: w.tick + 1 }; }
fn read_score(w: World) -> Int { return w.score; }
fn read_tick(w: World) -> Int { return w.tick; }
";

#[test]
fn struct_state_survives_hot_swap() {
    let mut rt = HotRuntime::new(&program(WORLD_V1));

    // state = init(); then two ticks under the original scoring rule.
    let mut state = rt.call("init", vec![]).unwrap();
    state = rt.call("step", vec![state, Value::Int(5)]).unwrap(); // score 5,  tick 1
    state = rt.call("step", vec![state, Value::Int(3)]).unwrap(); // score 8,  tick 2
    assert_eq!(as_int(rt.call("read_score", vec![state.clone()]).unwrap()), 8);

    // Swap the scoring rule. The World{score:8, tick:2} carries over.
    rt.hot_swap(&program(WORLD_V2));
    state = rt.call("step", vec![state, Value::Int(2)]).unwrap(); // score 8 + 2*10 = 28, tick 3

    assert_eq!(
        as_int(rt.call("read_score", vec![state.clone()]).unwrap()),
        28,
        "struct score carried across swap, new ×10 rule applied"
    );
    assert_eq!(
        as_int(rt.call("read_tick", vec![state]).unwrap()),
        3,
        "struct tick kept accumulating across the swap"
    );
}
