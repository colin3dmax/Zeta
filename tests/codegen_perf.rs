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
    let native = zeta::codegen::jit_run_i64_arg(&program, &[], "bench", 1000).unwrap();
    assert_eq!(native, oracle, "optimized native bench(1000) must match interpreter");
}

/// Compile the C bench with `flags`, run it with `n`, return (elapsed_ns, result).
fn run_c(flags: &[&str], n: i64) -> (i64, i64) {
    let dir = std::env::temp_dir();
    let c_path = dir.join("zeta_bench.c");
    let bin_path = dir.join(format!("zeta_bench_c_{}", flags.join("_").replace('-', "")));
    std::fs::File::create(&c_path)
        .unwrap()
        .write_all(C_BENCH.as_bytes())
        .unwrap();
    let ok = Command::new("cc")
        .args(flags)
        .arg("-o")
        .arg(&bin_path)
        .arg(&c_path)
        .status()
        .expect("cc should run")
        .success();
    assert!(ok, "C bench should compile with {flags:?}");
    let out = Command::new(&bin_path)
        .arg(n.to_string())
        .output()
        .expect("C bench should run");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut parts = stderr.split_whitespace();
    let ns: i64 = parts.next().unwrap().parse().unwrap();
    let result: i64 = parts.next().unwrap().parse().unwrap();
    (ns, result)
}

#[test]
#[ignore = "perf comparison; run with --ignored --nocapture"]
fn native_vs_c_hot_loop() {
    const N: i64 = 500_000_000;
    let program = zeta::lower_source(ZETA_BENCH).unwrap();

    // Zeta → LLVM → native (call timed, compilation excluded). Zeta Int is
    // WRAPPING (matches the interpreter), so codegen emits no `nsw`.
    let (native_result, native_dt) =
        zeta::codegen::jit_time_i64_arg(&program, &[], "bench", N).expect("native bench");
    let native_ns = native_dt.as_nanos() as f64;

    // C at -O2 exploits signed-overflow UB (assumes no wrap) — a transform Zeta's
    // defined wrapping semantics forbid. C at -O2 -fwrapv defines overflow as
    // wrapping, i.e. the SAME semantics as Zeta: the apples-to-apples baseline.
    let (c_o2_ns, c_o2_res) = run_c(&["-O2"], N);
    let (c_wrap_ns, c_wrap_res) = run_c(&["-O2", "-fwrapv"], N);

    assert_eq!(native_result, c_o2_res, "native and C(-O2) must agree");
    assert_eq!(native_result, c_wrap_res, "native and C(-fwrapv) must agree");

    let c_o2 = c_o2_ns as f64;
    let c_wrap = c_wrap_ns as f64;
    println!("\n=== hot Int loop, n={N} (result={native_result}) ===");
    println!("  C (-O2, UB no-wrap) : {:>9.2} ms  ({:.3} ns/iter)", c_o2 / 1e6, c_o2 / N as f64);
    println!("  C (-O2 -fwrapv)     : {:>9.2} ms  ({:.3} ns/iter)", c_wrap / 1e6, c_wrap / N as f64);
    println!("  Zeta→LLVM native    : {:>9.2} ms  ({:.3} ns/iter)", native_ns / 1e6, native_ns / N as f64);
    println!("  native / C(-O2)     : {:.2}x", native_ns / c_o2);
    println!("  native / C(-fwrapv) : {:.2}x  <- same (wrapping) semantics", native_ns / c_wrap);

    // The honest gate: against C with MATCHING (wrapping) semantics, native is
    // C-class. (Against -O2's UB-based transforms it may be slower — by design,
    // since Zeta's Int wraps.)
    assert!(
        native_ns / c_wrap < 1.5,
        "native is {:.2}x slower than C(-fwrapv) — should be ~1x at matching semantics",
        native_ns / c_wrap
    );
}
