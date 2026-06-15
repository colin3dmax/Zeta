//! Build a standalone `zetac-native` compiler binary using Zeta's OWN codegen.
//!
//! The self-hosting frontend (`testdata/selfhost/arena_frontend.zeta`) exposes
//! `compile(source, mode)`. We run it (interpreted) in `"llvm"` mode to emit
//! textual LLVM IR for *itself*, then link that IR with a tiny file-IO C driver
//! into a real executable. The resulting binary is the Zeta compiler compiled to
//! native code by the Zeta compiler — no inkwell, no Rust-side codegen.
//!
//!   cargo run --release --example build_zetac
//!   ./zetac-native <file.zeta> <ast-dump|mir-dump|resolve|typecheck|run>
//!
//! Needs `clang` (set `LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm`, else falls
//! back to `clang` on PATH).

use std::process::Command;

const FRONTEND: &str = include_str!("../testdata/selfhost/arena_frontend.zeta");

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

fn clang_path() -> String {
    match std::env::var("LLVM_SYS_221_PREFIX") {
        Ok(prefix) => format!("{prefix}/bin/clang"),
        Err(_) => "clang".to_string(),
    }
}

fn source_file(path: &str, source: &str) -> zeta::module_graph::SourceFile {
    zeta::module_graph::SourceFile {
        path: path.to_string(),
        source: source.to_string(),
    }
}

fn main() {
    eprintln!("[1/3] emitting LLVM IR for the frontend via its own codegen (interpreted, ~70s)...");
    let caller = format!(
        "module selfhost.caller;\nimport selfhost.arena_frontend;\n\nfn main() -> String {{\n  let source: String = {lit};\n  return selfhost.arena_frontend.compile(source, \"llvm\");\n}}\n",
        lit = zeta_string_literal(FRONTEND),
    );
    let ir = zeta::module_graph::run_sources(&[
        source_file("testdata/selfhost/arena_frontend.zeta", FRONTEND),
        source_file("testdata/selfhost/caller.zeta", &caller),
    ])
    .expect("frontend should emit IR")
    .to_string();
    eprintln!("      emitted {} KB of IR", ir.len() / 1024);

    let out_dir = std::path::Path::new("target");
    std::fs::create_dir_all(out_dir).expect("target dir");
    let ll = out_dir.join("zetac_frontend.ll");
    let driver = out_dir.join("zetac_driver.c");
    std::fs::write(&ll, &ir).expect("write ll");
    std::fs::write(&driver, DRIVER_C).expect("write driver");

    eprintln!("[2/3] linking with clang...");
    let exe = "zetac-native";
    let build = Command::new(clang_path())
        .arg("-O2")
        .arg(&ll)
        .arg(&driver)
        .arg("-o")
        .arg(exe)
        .output()
        .expect("invoke clang");
    if !build.status.success() {
        eprintln!("clang failed:\n{}", String::from_utf8_lossy(&build.stderr));
        std::process::exit(1);
    }

    let size = std::fs::metadata(exe).map(|m| m.len() / 1024).unwrap_or(0);
    eprintln!("[3/3] done: ./{exe} ({size} KB)");
    eprintln!("       run:  ./{exe} <file.zeta> <ast-dump|mir-dump|resolve|typecheck|run>");
}
