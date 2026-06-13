//! Stage1/Rust ast-dump 差分对齐工具。
//!
//! 用 Rust 的 `dump_ast`(只 parse 不 typecheck)作为 oracle,与 Zeta 自举的
//! Stage1 前端 `ast_dump_rust_item_dump` 输出逐行比对,快速定位 parity 缺口。
//!
//! 用法:
//!   cargo run --quiet --example parity_check -- <file.zeta> [more.zeta ...]
//!
//! 退出码:全部对齐返回 0,任一 MISMATCH 或 oracle 解析失败返回 1。

use std::process::ExitCode;

const FRONTEND: &str = include_str!("../testdata/stage1_frontend/frontend.zeta");

fn stage1_rust_item_dump(source: &str) -> String {
    let escaped = source
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    let stage2_app = format!(
        r#"
module stage2.parity;
import stage1.frontend;

fn main() -> String {{
  let source: String = "{escaped}";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}}
"#
    );
    let value = zeta::module_graph::run_sources(&[
        zeta::module_graph::SourceFile {
            path: "testdata/stage1_frontend/frontend.zeta".to_string(),
            source: FRONTEND.to_string(),
        },
        zeta::module_graph::SourceFile {
            path: "testdata/stage1_parity/__probe.zeta".to_string(),
            source: stage2_app,
        },
    ])
    .expect("Stage1 parity harness should run the frontend");
    value.to_string()
}

fn first_diff_line(a: &str, b: &str) -> Option<(usize, String, String)> {
    let mut la = a.lines();
    let mut lb = b.lines();
    let mut idx = 0;
    loop {
        match (la.next(), lb.next()) {
            (Some(x), Some(y)) => {
                if x != y {
                    return Some((idx, x.to_string(), y.to_string()));
                }
            }
            (None, None) => return None,
            (x, y) => {
                return Some((
                    idx,
                    x.unwrap_or("<eof>").to_string(),
                    y.unwrap_or("<eof>").to_string(),
                ));
            }
        }
        idx += 1;
    }
}

fn main() -> ExitCode {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        eprintln!("用法: cargo run --example parity_check -- <file.zeta> ...");
        return ExitCode::FAILURE;
    }

    let mut failures = 0;
    for path in &files {
        let source = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                println!("READ_ERROR {path}: {e}");
                failures += 1;
                continue;
            }
        };

        let oracle = match zeta::dump_ast(&source) {
            Ok(s) => s,
            Err(diags) => {
                println!(
                    "ORACLE_PARSE_FAIL {path}: Rust parser 拒绝了该源码 ({} 个诊断)",
                    diags.len()
                );
                failures += 1;
                continue;
            }
        };

        let stage1 = stage1_rust_item_dump(&source);

        if stage1.trim_end() == oracle.trim_end() {
            println!("PASS {path}");
        } else {
            failures += 1;
            match first_diff_line(stage1.trim_end(), oracle.trim_end()) {
                Some((line, got, want)) => {
                    println!("MISMATCH {path} @line {line}");
                    println!("  stage1: {got}");
                    println!("  rust  : {want}");
                }
                None => println!("MISMATCH {path} (仅尾部差异)"),
            }
        }
    }

    println!("---\n{} 个文件, {} 个失败", files.len(), failures);
    if failures == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
