// Stage2 (slice 1) differential gate: the Zeta-side LLVM IR emitter.
//
// `testdata/selfhost/arena_frontend.zeta` now exposes `compile(src, "llvm")`,
// which walks the MIR arena and emits textual LLVM IR for the scalar subset
// (Int/Bool as i64; let/assign/return; arithmetic/bitwise/comparison/unary/
// logical; if/else/while/break/continue; user calls incl. recursion). This gate
// runs that emitter inside the Stage0 interpreter, compiles the emitted IR with
// clang, links a tiny C driver that prints `z_main()`, runs it, and asserts the
// printed result equals the Stage0 interpreter oracle (`run_mir`) for the same
// program. So: the Zeta-emitted native code reproduces the interpreter exactly.
//
// Needs the LLVM toolchain, so it is gated behind the `llvm` cargo feature and
// run alongside the other codegen_* gates:
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test selfhost_llvm
#![cfg(feature = "llvm")]

use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

const FRONTEND_SOURCE: &str = include_str!("../testdata/selfhost/arena_frontend.zeta");

fn source_file(path: &str, source: &str) -> zeta::module_graph::SourceFile {
    zeta::module_graph::SourceFile {
        path: path.to_string(),
        source: source.to_string(),
    }
}

/// Escape a Zeta source so it can be embedded inside a double-quoted Zeta
/// string literal in the caller app.
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

/// Run `compile(program_source, "llvm")` inside the Zeta interpreter and return
/// the textual LLVM IR it produces.
fn emit_ir(program_source: &str) -> String {
    let caller = format!(
        r#"
module selfhost.caller;
import selfhost.arena_frontend;

fn main() -> String {{
  let source: String = {literal};
  return selfhost.arena_frontend.compile(source, "llvm");
}}
"#,
        literal = zeta_string_literal(program_source),
    );
    let value = zeta::module_graph::run_sources(&[
        source_file("testdata/selfhost/arena_frontend.zeta", FRONTEND_SOURCE),
        source_file("testdata/selfhost/caller.zeta", &caller),
    ])
    .expect("llvm emit caller should run");
    value.to_string()
}

/// The Stage0 interpreter oracle: lower + run the program, expecting an Int.
fn oracle(program_source: &str) -> i64 {
    let program = zeta::lower_source(program_source).expect("oracle should lower");
    match zeta::runtime::run_mir(&program).expect("oracle should run") {
        zeta::runtime::Value::Int(n) => n,
        other => panic!("oracle produced non-Int: {other:?}"),
    }
}

fn clang_path() -> String {
    match std::env::var("LLVM_SYS_221_PREFIX") {
        Ok(prefix) => format!("{prefix}/bin/clang"),
        Err(_) => "clang".to_string(),
    }
}

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Compile the emitted IR with clang, link a C driver that prints `z_main()`,
/// run it, and return the printed i64.
fn run_native(ir: &str) -> i64 {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("zeta_llvm_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).expect("temp dir");

    let ll = dir.join("entry.ll");
    std::fs::File::create(&ll)
        .and_then(|mut f| f.write_all(ir.as_bytes()))
        .expect("write .ll");

    let driver = dir.join("driver.c");
    std::fs::File::create(&driver)
        .and_then(|mut f| {
            f.write_all(b"#include <stdio.h>\nlong z_main(void);\nint main(void){ printf(\"%ld\\n\", z_main()); return 0; }\n")
        })
        .expect("write driver.c");

    let exe = dir.join("prog");
    let compile = Command::new(clang_path())
        .arg(&ll)
        .arg(&driver)
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("invoke clang");
    assert!(
        compile.status.success(),
        "clang failed to compile emitted IR:\n--- stderr ---\n{}\n--- IR ---\n{ir}",
        String::from_utf8_lossy(&compile.stderr),
    );

    let run = Command::new(&exe).output().expect("run native exe");
    assert!(run.status.success(), "native exe exited with failure");
    let stdout = String::from_utf8_lossy(&run.stdout);
    let result = stdout
        .trim()
        .parse::<i64>()
        .unwrap_or_else(|_| panic!("native printed non-integer: {stdout:?}"));

    let _ = std::fs::remove_dir_all(&dir);
    result
}

/// Assert the Zeta-emitted native code matches the interpreter oracle.
fn check(program_source: &str) -> i64 {
    let want = oracle(program_source);
    let ir = emit_ir(program_source);
    let got = run_native(&ir);
    assert_eq!(
        got, want,
        "native/interpreter divergence\n--- source ---\n{program_source}\n--- IR ---\n{ir}\n--- native={got} oracle={want} ---"
    );
    want
}

#[test]
fn let_and_arithmetic() {
    assert_eq!(check("fn main() -> Int { let x: Int = 40; return x + 2; }"), 42);
}

#[test]
fn arithmetic_and_bitwise_precedence() {
    let src = "fn main() -> Int { return (7 * 6) - (10 / 2) % 3 + (12 & 6) | 1; }";
    check(src);
}

#[test]
fn comparison_into_if() {
    let src = "fn main() -> Int { let a: Int = 5; if a > 3 { return 1; } return 0; }";
    assert_eq!(check(src), 1);
}

#[test]
fn logical_not_and_comparison() {
    let src = "fn main() -> Int { let b: Bool = 3 > 5; if !b { return 7; } return 0; }";
    assert_eq!(check(src), 7);
}

#[test]
fn if_else_chain_via_calls() {
    let src = "\
fn classify(n: Int) -> Int {
  if n < 0 { return 0 - 1; }
  if n == 0 { return 0; }
  return 1;
}
fn main() -> Int {
  return classify(0 - 7) * 100 + classify(0) * 10 + classify(9);
}";
    check(src);
}

#[test]
fn while_loop_sum() {
    let src = "\
fn main() -> Int {
  let mut s: Int = 0;
  let mut i: Int = 0;
  while i < 10 { s = s + i; i = i + 1; }
  return s;
}";
    assert_eq!(check(src), 45);
}

#[test]
fn recursive_fib() {
    let src = "\
fn fib(n: Int) -> Int {
  if n < 2 { return n; }
  return fib(n - 1) + fib(n - 2);
}
fn main() -> Int { return fib(20); }";
    assert_eq!(check(src), 6765);
}

#[test]
fn nested_loops() {
    let src = "\
fn main() -> Int {
  let mut acc: Int = 0;
  let mut i: Int = 0;
  while i < 5 {
    let mut j: Int = 0;
    while j < 5 { acc = acc + 1; j = j + 1; }
    i = i + 1;
  }
  return acc;
}";
    assert_eq!(check(src), 25);
}

#[test]
fn break_and_continue() {
    let src = "\
fn main() -> Int {
  let mut s: Int = 0;
  let mut i: Int = 0;
  while i < 100 {
    i = i + 1;
    if i > 10 { break; }
    if i % 2 == 0 { continue; }
    s = s + i;
  }
  return s;
}";
    // odds 1..=9: 1+3+5+7+9
    assert_eq!(check(src), 25);
}

#[test]
fn logical_and_or() {
    let src = "\
fn main() -> Int {
  let a: Int = 5;
  let mut r: Int = 0;
  if a > 0 && a < 10 { r = 1; }
  if a > 100 || a == 5 { r = r + 10; }
  return r;
}";
    assert_eq!(check(src), 11);
}
