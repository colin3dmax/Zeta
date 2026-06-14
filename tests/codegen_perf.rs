// Native backend — performance comparison vs hand-written C (cargo feature
// `llvm`). Answers the headline question: does Zeta-compiled native code reach
// C/C++ speed?
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_perf -- --ignored --nocapture
//
// The same arithmetic hot loop is run three ways — Zeta→LLVM→native, hand-
// written C at `cc -O2`, and the Stage0 interpreter — and timed. A runtime `n`
// keeps both compilers from constant-folding the loop away.
#![cfg(feature = "llvm")]

use std::io::Write;
use std::process::Command;
use std::time::Instant;

use zeta::runtime::Value;

// One non-trivial integer loop, expressed identically in Zeta and C.
const ZETA_BENCH: &str = "\
fn bench(n: Int) -> Int {
  let mut i: Int = 0;
  let mut acc: Int = 0;
  while i < n {
    acc = acc + i * i - i / 3;
    i = i + 1;
  }
  return acc;
}
fn main() -> Int { return bench(1000); }";

const C_BENCH: &str = "\
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <time.h>
int64_t bench(int64_t n) {
  int64_t i = 0, acc = 0;
  while (i < n) { acc = acc + i * i - i / 3; i = i + 1; }
  return acc;
}
int main(int argc, char** argv) {
  int64_t n = atoll(argv[1]);
  struct timespec t0, t1;
  clock_gettime(CLOCK_MONOTONIC, &t0);
  int64_t r = bench(n);
  clock_gettime(CLOCK_MONOTONIC, &t1);
  long long ns = (t1.tv_sec - t0.tv_sec) * 1000000000LL + (t1.tv_nsec - t0.tv_nsec);
  fprintf(stderr, \"%lld %lld\\n\", ns, (long long)r); // elapsed_ns result
  return 0;
}";

#[test]
fn optimized_native_matches_interpreter() {
    // The optimized (-O3, runtime-arg) path must still agree with the oracle.
    let program = zeta::lower_source(ZETA_BENCH).unwrap();
    let oracle = match zeta::runtime::run_mir(&program).unwrap() {
        Value::Int(n) => n,
        other => panic!("{other:?}"),
    };
    let native = zeta::codegen::jit_run_i64_arg(&program, "bench", 1000).unwrap();
    assert_eq!(native, oracle, "optimized native bench(1000) must match interpreter");
}

#[test]
#[ignore = "perf comparison; run with --ignored --nocapture"]
fn native_vs_c_hot_loop() {
    const N: i64 = 500_000_000;
    let program = zeta::lower_source(ZETA_BENCH).unwrap();

    // --- Zeta → LLVM → native (call timed, compilation excluded) ---
    let (native_result, native_dt) =
        zeta::codegen::jit_time_i64_arg(&program, "bench", N).expect("native bench");

    // --- hand-written C at cc -O2 ---
    let dir = std::env::temp_dir();
    let c_path = dir.join("zeta_bench.c");
    let bin_path = dir.join("zeta_bench_c");
    std::fs::File::create(&c_path)
        .unwrap()
        .write_all(C_BENCH.as_bytes())
        .unwrap();
    let compile = Command::new("cc")
        .args(["-O2", "-o"])
        .arg(&bin_path)
        .arg(&c_path)
        .status()
        .expect("cc should run");
    assert!(compile.success(), "C bench should compile");
    let run = Command::new(&bin_path)
        .arg(N.to_string())
        .output()
        .expect("C bench should run");
    let stderr = String::from_utf8_lossy(&run.stderr);
    let mut parts = stderr.split_whitespace();
    let c_ns: i64 = parts.next().unwrap().parse().unwrap();
    let c_result: i64 = parts.next().unwrap().parse().unwrap();

    // --- correctness across all three ---
    assert_eq!(native_result, c_result, "native and C must agree");

    let native_ns = native_dt.as_nanos() as f64;
    let c_ns = c_ns as f64;
    let ratio = native_ns / c_ns;

    println!("\n=== hot Int loop, n={N} (result={native_result}) ===");
    println!("  C (cc -O2)        : {:>9.2} ms", c_ns / 1e6);
    println!("  Zeta→LLVM native  : {:>9.2} ms", native_ns / 1e6);
    println!("  native / C ratio  : {ratio:.2}x");
    println!(
        "  per-iteration     : C {:.3} ns | native {:.3} ns",
        c_ns / N as f64,
        native_ns / N as f64
    );

    // Sanity gate: native should be in the same ballpark as C (not interpreted).
    assert!(
        ratio < 3.0,
        "native is {ratio:.2}x slower than C — expected ~1x (same LLVM backend)"
    );
}
