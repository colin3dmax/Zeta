// Standalone self-hosted binary, built by Zeta's OWN codegen.
//
// This is the full payoff of the Stage2 capstone: `arena_frontend.zeta`'s
// `compile(source, mode)` is emitted to LLVM IR by the Zeta-side codegen
// (`compile(frontend, "llvm")`), linked with a tiny file-IO C driver into a
// real `zetac-native` executable, and run as `./zetac-native <file.zeta> <mode>`
// on a real source file. Its output must match the Stage0 reference byte for
// byte — i.e. the Zeta compiler, compiled to native by itself, is a usable
// standalone tool. (Mirrors the Rust-backend `codegen_aot_frontend.rs`, but the
// native code here is produced by Zeta's own emitter, not inkwell.)
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test selfhost_aot -- --ignored
#![cfg(feature = "llvm")]

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

const FRONTEND_SOURCE: &str = include_str!("../testdata/selfhost/arena_frontend.zeta");

fn source_file(path: &str, source: &str) -> zeta::module_graph::SourceFile {
    zeta::module_graph::SourceFile {
        path: path.to_string(),
        source: source.to_string(),
    }
}

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

/// Emit textual LLVM IR for `program_source` via the Zeta-side codegen.
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

fn clang_path() -> String {
    match std::env::var("LLVM_SYS_221_PREFIX") {
        Ok(prefix) => format!("{prefix}/bin/clang"),
        Err(_) => "clang".to_string(),
    }
}

// The file-IO driver: read a .zeta file, call the natively-compiled frontend
// entry `z_compile(source, mode)`, write its String result to stdout.
const DRIVER_C: &str = r#"#include <stdio.h>
#include <stdlib.h>
#include <string.h>
typedef struct { long long len; char* ptr; } ZStr;
extern ZStr z_compile(ZStr source, ZStr mode);
static ZStr read_file(const char* path) {
  FILE* f = fopen(path, "rb");
  if (!f) { fprintf(stderr, "cannot open %s\n", path); exit(1); }
  fseek(f, 0, SEEK_END); long n = ftell(f); fseek(f, 0, SEEK_SET);
  char* buf = (char*) malloc(n > 0 ? n : 1);
  if (n > 0 && fread(buf, 1, n, f) != (size_t) n) { exit(1); }
  fclose(f);
  ZStr s; s.len = n; s.ptr = buf; return s;
}
int main(int argc, char** argv) {
  if (argc < 3) { fprintf(stderr, "usage: %s <file.zeta> <mode>\n", argv[0]); return 2; }
  ZStr src = read_file(argv[1]);
  ZStr mode; mode.len = (long long) strlen(argv[2]); mode.ptr = argv[2];
  ZStr out = z_compile(src, mode);
  fwrite(out.ptr, 1, (size_t) out.len, stdout);
  return 0;
}
"#;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

#[test]
#[ignore]
fn standalone_zetac_binary() {
    // Build the zetac-native binary from Zeta-emitted IR + the file-IO driver.
    let ir = emit_ir(FRONTEND_SOURCE);
    assert!(ir.len() > 50_000, "emitted frontend IR too small: {}", ir.len());

    let id = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("zetac_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(dir.join("frontend.ll"), &ir).expect("write ll");
    std::fs::write(dir.join("driver.c"), DRIVER_C).expect("write driver");

    let exe = dir.join("zetac-native");
    let build = Command::new(clang_path())
        .arg("-O2")
        .arg(dir.join("frontend.ll"))
        .arg(dir.join("driver.c"))
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("invoke clang");
    assert!(
        build.status.success(),
        "clang failed to build zetac-native:\n{}",
        String::from_utf8_lossy(&build.stderr),
    );

    // A real source file with structs, control flow, arrays, and recursion.
    let sample = "\
import std.core;
struct Point { x: Int, y: Int }
fn dist2(p: Point) -> Int { return p.x * p.x + p.y * p.y; }
fn sum_to(n: Int) -> Int {
  let mut s: Int = 0;
  let mut i: Int = 0;
  while i < n { s = s + i; i = i + 1; }
  return s;
}
fn fib(n: Int) -> Int {
  if n < 2 { return n; }
  return fib(n - 1) + fib(n - 2);
}
fn main() -> Int {
  let p: Point = Point { x: 3, y: 4 };
  return dist2(p) + sum_to(10) + fib(7);
}
";
    let src_path = dir.join("sample.zeta");
    std::fs::write(&src_path, sample).expect("write sample");

    let run_mode = |mode: &str| -> String {
        let out = Command::new(&exe)
            .arg(&src_path)
            .arg(mode)
            .output()
            .expect("run zetac-native");
        assert!(out.status.success(), "zetac-native {mode} failed");
        String::from_utf8_lossy(&out.stdout).to_string()
    };

    // Each mode's output must match the Stage0 reference byte-for-byte.
    assert_eq!(
        run_mode("ast-dump").trim_end(),
        zeta::dump_ast(sample).expect("oracle ast-dump").trim_end(),
        "standalone zetac ast-dump diverged",
    );
    assert_eq!(
        run_mode("mir-dump").trim_end(),
        zeta::dump_mir(sample).expect("oracle mir-dump").trim_end(),
        "standalone zetac mir-dump diverged",
    );
    assert_eq!(
        run_mode("run").trim_end(),
        zeta::run_source(sample).expect("oracle run").to_string().trim_end(),
        "standalone zetac run diverged",
    );
    // resolve / typecheck on clean source report nothing.
    assert_eq!(run_mode("resolve").trim_end(), "", "resolve should be clean");
    assert_eq!(run_mode("typecheck").trim_end(), "", "typecheck should be clean");

    eprintln!(
        "built standalone zetac-native ({} KB exe) — ran ast-dump/mir-dump/run/resolve/typecheck on a real .zeta file, all byte-correct",
        std::fs::metadata(&exe).map(|m| m.len() / 1024).unwrap_or(0),
    );
    let _ = std::fs::remove_dir_all(&dir);
}
