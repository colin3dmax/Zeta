// M7 slice 3 (perf): scaling measurement / regression for the Zeta frontend
// running inside the Stage0 MIR interpreter.
//
// The arena frontend threads a `ParseResult { arena, .. }` through every
// sub-parser (`let r = parse_x(a, ..); a = r.arena`), which keeps the caller's
// `a` live across the call. Under the interpreter's copy-on-write `Value`, that
// shared liveness forces each `push` inside a sub-parser to `Rc::make_mut`-clone
// the whole backing array — the residual O(n^2).
//
// `measure` runs `dump_module_via_arena` over a synthetic source of `k`
// functions and returns wall-clock millis. Source size n grows linearly in k,
// so a quadratic frontend shows time ∝ k^2 (i.e. time/k grows linearly in k),
// while a linear frontend shows time/k roughly flat.
//
// The `perf_scaling_report` test is `#[ignore]` (it prints the curve for human
// inspection). The `perf_scaling_is_subquadratic` test is the automated
// regression gate: it asserts the per-function cost does not blow up
// quadratically as k doubles.

use std::time::Instant;

fn source_file(path: &str, source: &str) -> zeta::module_graph::SourceFile {
    zeta::module_graph::SourceFile {
        path: path.to_string(),
        source: source.to_string(),
    }
}

const FRONTEND_SOURCE: &str = include_str!("../testdata/selfhost/arena_frontend.zeta");

fn zeta_string_literal(source: &str) -> String {
    let mut out = String::from("\"");
    for ch in source.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// A synthetic program of `k` small functions — each contributes a fixed number
/// of tokens, so total source size grows linearly in `k`.
fn synthetic_program(k: usize) -> String {
    let mut src = String::new();
    for i in 0..k {
        src.push_str(&format!(
            "fn f{i}(a: Int, b: Int) -> Int {{ let x: Int = a + b * 2; let y: Int = x - 1; return x + y; }}\n"
        ));
    }
    src
}

/// Run `dump_module_via_arena` over an k-function synthetic source through the
/// interpreter and return elapsed milliseconds.
fn measure(k: usize) -> u128 {
    let program = synthetic_program(k);
    let caller = format!(
        r#"
module selfhost.caller;
import selfhost.arena_frontend;

fn main() -> String {{
  let source: String = {literal};
  return selfhost.arena_frontend.dump_module_via_arena(source);
}}
"#,
        literal = zeta_string_literal(&program),
    );

    let started = Instant::now();
    let value = zeta::module_graph::run_sources(&[
        source_file("testdata/selfhost/arena_frontend.zeta", FRONTEND_SOURCE),
        source_file("testdata/selfhost/caller.zeta", &caller),
    ])
    .expect("frontend caller should run");
    let elapsed = started.elapsed().as_millis();
    // Touch the result so the run is not optimized away.
    assert!(!value.to_string().is_empty());
    elapsed
}

#[test]
#[ignore = "prints a human-readable scaling curve; run with --ignored --nocapture"]
fn perf_scaling_report() {
    println!("\n  k |   ms | ms/k");
    println!("----+------+------");
    for k in [8usize, 16, 32, 64, 128] {
        let ms = measure(k);
        println!("{k:3} | {ms:4} | {:.3}", ms as f64 / k as f64);
    }
}

#[test]
fn perf_scaling_is_subquadratic() {
    // Total-time ratio across an 8x input increase (k=16 → k=128). A linear
    // frontend gives ~8x; a quadratic one gives ~64x. Before the move-on-last-
    // use optimization this ratio was ~19x (the residual O(n^2)); after it, ~8x
    // (essentially linear). The threshold of 12 sits well above linear (≈50%
    // slack for interpreter constant factors / CI noise) while still catching a
    // regression back toward quadratic.
    let small_k = 16usize;
    let large_k = 128usize;

    // Warm up to avoid first-run allocation noise dominating the measurement.
    let _ = measure(small_k);
    let _ = measure(large_k);

    let small_ms = measure(small_k).max(1);
    let large_ms = measure(large_k).max(1);

    let ratio = large_ms as f64 / small_ms as f64;
    assert!(
        ratio < 12.0,
        "frontend time grew {ratio:.1}x across an 8x input increase \
         (k={small_k}: {small_ms}ms → k={large_k}: {large_ms}ms) — expected ~8x (linear); \
         a return toward quadratic (~19x+) means the move-on-last-use optimization regressed"
    );
}
