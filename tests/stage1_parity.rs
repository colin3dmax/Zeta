//! Stage1 自举前端 与 Rust ast-dump 的差分对齐回归门禁。
//!
//! `testdata/stage1_parity/` 下的每个 `.zeta` 探针都会被两条路径处理:
//!   - oracle:Rust 的 `dump_ast`(只 parse,不 typecheck)
//!   - actual:Zeta 自举的 Stage1 前端 `ast_dump_rust_item_dump`
//! 二者必须逐字相等(忽略尾部空白),从而把 Stage1/Rust 的 parity 固化为回归测试。
//!
//! 新增对齐用例 = 往 `testdata/stage1_parity/` 丢一个 `.zeta` 文件即可,
//! 无需手抄 golden,也无需改动本文件。

use std::fs;
use std::path::PathBuf;

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

fn probe_files() -> Vec<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("testdata/stage1_parity");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("stage1_parity 探针目录应存在")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|e| e == "zeta").unwrap_or(false))
        .collect();
    files.sort();
    files
}

#[test]
fn stage1_parity_probes_match_rust_oracle() {
    let files = probe_files();
    assert!(
        !files.is_empty(),
        "testdata/stage1_parity/ 下至少应有一个探针"
    );

    let mut mismatches: Vec<String> = Vec::new();
    for path in &files {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let source = fs::read_to_string(path).expect("应能读取探针源码");

        let oracle = match zeta::dump_ast(&source) {
            Ok(s) => s,
            Err(diags) => {
                mismatches.push(format!(
                    "{name}: Rust oracle 解析失败 ({} 个诊断)",
                    diags.len()
                ));
                continue;
            }
        };
        let stage1 = stage1_rust_item_dump(&source);

        if stage1.trim_end() != oracle.trim_end() {
            mismatches.push(format!(
                "{name}: Stage1 与 Rust ast-dump 不一致\n--- stage1 ---\n{}\n--- rust ---\n{}",
                stage1.trim_end(),
                oracle.trim_end()
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "{} 个 parity 探针失配:\n\n{}",
        mismatches.len(),
        mismatches.join("\n\n")
    );
}
