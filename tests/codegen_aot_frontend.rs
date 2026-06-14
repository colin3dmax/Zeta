// Native backend — STANDALONE AOT frontend: compile the whole Zeta self-hosting
// frontend to a native object, link it with a tiny C driver into a real
// executable, and run `./frontend <source-file> <mode>`. The C driver is the
// file-IO shim (reads the source file, calls the AOT'd `compile`, writes the
// resulting dump to stdout). Its output must match the Stage0 interpreter's
// `compile(source, mode)` byte-for-byte — Zeta's frontend, compiled by Zeta's
// own native backend, running as a standalone binary off Stage0.
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_aot_frontend -- --ignored --nocapture
#![cfg(feature = "llvm")]

use std::io::Write;
use std::process::Command;

use zeta::ast::Item;
use zeta::runtime::Value;

const FRONTEND: &str = include_str!("../testdata/selfhost/arena_frontend.zeta");

// The file-IO shim + ABI bridge. A Zeta `String` is `{ i64 len, ptr data }`
// (16 bytes), passed/returned in registers per AAPCS64 — same as C's struct
// here (and the same layout the NativeArray FFI already relies on).
const DRIVER: &str = "\
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
typedef struct { long long len; char* ptr; } ZStr;
extern ZStr zeta_entry(ZStr source, ZStr mode);
static ZStr read_file(const char* path) {
  FILE* f = fopen(path, \"rb\");
  if (!f) { fprintf(stderr, \"cannot open %s\\n\", path); exit(1); }
  fseek(f, 0, SEEK_END); long n = ftell(f); fseek(f, 0, SEEK_SET);
  char* buf = (char*) malloc(n > 0 ? n : 1);
  if (n > 0 && fread(buf, 1, n, f) != (size_t) n) { exit(1); }
  fclose(f);
  ZStr s; s.len = n; s.ptr = buf; return s;
}
int main(int argc, char** argv) {
  if (argc < 3) { fprintf(stderr, \"usage: %s <file> <mode>\\n\", argv[0]); return 2; }
  ZStr src = read_file(argv[1]);
  ZStr mode; mode.len = (long long) strlen(argv[2]); mode.ptr = argv[2];
  ZStr out = zeta_entry(src, mode);
  fwrite(out.ptr, 1, (size_t) out.len, stdout);
  return 0;
}
";

/// Escape `s` for embedding inside a double-quoted Zeta string literal.
fn zeta_string_literal(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// Interpreter oracle: `compile(sample, mode)` as a String, via a `main` that
/// returns the frontend's result directly.
fn interp_compile(sample: &str, mode: &str) -> String {
    let caller = format!(
        "{FRONTEND}\n\nfn main() -> String {{ let src: String = {}; return compile(src, \"{mode}\"); }}\n",
        zeta_string_literal(sample),
    );
    let program = zeta::lower_source(&caller).expect("caller should lower");
    match zeta::runtime::run_mir(&program).expect("interpreter should run") {
        Value::String(s) => s,
        other => panic!("expected String from compile, got {other:?}"),
    }
}

const SAMPLE: &str = "\
struct Point { x: Int, y: Int }
fn sum_to(n: Int) -> Int {
  let mut s: Int = 0;
  for i in 0..n { s = s + i; }
  return s;
}
fn main() -> Int {
  let p: Point = Point { x: 3, y: 4 };
  let xs: IntArray = [10, 20, 30];
  let mut total: Int = p.x + p.y;
  for v in xs { total = total + v; }
  if total > 50 { return sum_to(total); }
  return total;
}
";

#[test]
#[ignore] // builds + O3-optimizes the whole 306-fn frontend; run explicitly.
fn aot_frontend_standalone_binary() {
    // 1. AOT-compile the frontend's `compile` entry to a native object (once).
    let module = zeta::parse_source(FRONTEND).expect("frontend should parse");
    let structs: Vec<zeta::ast::StructDecl> = module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(decl) => Some(decl.clone()),
            _ => None,
        })
        .collect();
    let program = zeta::lower_source(FRONTEND).expect("frontend should lower");

    let dir = std::env::temp_dir();
    let obj = dir.join("zeta_frontend.o");
    let drv = dir.join("zeta_frontend_driver.c");
    let exe = dir.join("zeta_frontend");
    let srcfile = dir.join("zeta_frontend_sample.zeta");

    zeta::codegen::aot_compile_object(&program, &structs, "compile", &obj)
        .expect("AOT object for the frontend");
    std::fs::File::create(&drv).unwrap().write_all(DRIVER.as_bytes()).unwrap();
    std::fs::File::create(&srcfile).unwrap().write_all(SAMPLE.as_bytes()).unwrap();

    let linked = Command::new("cc")
        .arg(&obj)
        .arg(&drv)
        .arg("-o")
        .arg(&exe)
        .status()
        .expect("cc link");
    assert!(linked.success(), "linking the AOT frontend should succeed");

    // 2. Run the standalone binary for each mode and diff against the interpreter.
    for mode in ["ast-dump", "mir-dump", "typecheck", "run"] {
        let out = Command::new(&exe)
            .arg(&srcfile)
            .arg(mode)
            .output()
            .expect("run frontend exe");
        assert!(out.status.success(), "frontend exe should run for mode `{mode}`");
        let got = String::from_utf8_lossy(&out.stdout).to_string();
        let want = interp_compile(SAMPLE, mode);
        assert_eq!(
            got, want,
            "AOT frontend / interpreter divergence for mode `{mode}`\n--- got ---\n{got}\n--- want ---\n{want}\n"
        );
        println!("mode {mode}: standalone AOT binary == interpreter ({} bytes)", got.len());
    }
}
