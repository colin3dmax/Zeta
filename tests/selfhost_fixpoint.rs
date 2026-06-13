// M7 self-hosting milestone (slice 1): the fixpoint gate. The Zeta-written
// frontend/checker/lowering pipeline (testdata/selfhost/arena_frontend.zeta)
// must be able to process ITS OWN SOURCE and agree with the Rust oracle on
// every stage:
//
//   * ast-dump(self)      == zeta::dump_ast(self)
//   * resolve_report(self)   is empty, and the Rust resolver agrees (clean)
//   * typecheck_report(self) is empty, and the Rust typechecker agrees (clean)
//   * mir-dump(self)      == zeta::dump_mir(self)
//
// This is the strongest single-interpretation-level self-application: the
// compiler frontend written in Zeta handles the full 7.5k-line source of
// itself, byte-for-byte identical to Stage0.
//
// These tests are `#[ignore]` by default: running the Zeta frontend over its
// own 7.5k-line source inside the tree-walking interpreter takes ~3 minutes
// (release) / ~6 minutes (debug), too slow for routine `cargo test`. Run the
// capstone gate explicitly with:
//
//     cargo test --release --test selfhost_fixpoint -- --ignored
//
// The fast `selfhost_arena`/`selfhost_resolve`/`selfhost_typecheck`/
// `selfhost_mir` parity probes (250+ cases) guard the frontend's correctness
// on every `cargo test`; this gate is the additional on-demand whole-program
// self-application proof.
//
// Performance note: arena threading through `ParseResult` (`let r =
// parse_x(a, ..); a = r.arena`) keeps `a` live across the call, so each push
// inside a sub-parser still copy-on-writes its backing array — the residual
// O(n^2). The `Rc` copy-on-write `Value` already removed the dominant
// whole-struct clone-per-field-read blowup; driving this to O(n) would need
// last-use/liveness-driven moves in the interpreter (future work).

fn source_file(path: &str, source: &str) -> zeta::module_graph::SourceFile {
    zeta::module_graph::SourceFile {
        path: path.to_string(),
        source: source.to_string(),
    }
}

const FRONTEND_SOURCE: &str = include_str!("../testdata/selfhost/arena_frontend.zeta");

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

/// Run one exported entry point of the arena frontend over `program_source`
/// inside the Zeta interpreter and return the string it produces.
fn run_entry(entry: &str, program_source: &str) -> String {
    let caller = format!(
        r#"
module selfhost.caller;
import selfhost.arena_frontend;

fn main() -> String {{
  let source: String = {literal};
  return selfhost.arena_frontend.{entry}(source);
}}
"#,
        literal = zeta_string_literal(program_source),
    );

    let value = zeta::module_graph::run_sources(&[
        source_file("testdata/selfhost/arena_frontend.zeta", FRONTEND_SOURCE),
        source_file("testdata/selfhost/caller.zeta", &caller),
    ])
    .expect("arena frontend caller should run");

    value.to_string()
}

#[test]
#[ignore = "slow capstone gate; run with `cargo test --release -- --ignored`"]
fn fixpoint_ast_dump_of_self_matches_oracle() {
    let oracle = zeta::dump_ast(FRONTEND_SOURCE).expect("oracle ast-dump of self should succeed");
    let arena = run_entry("dump_module_via_arena", FRONTEND_SOURCE);
    assert_eq!(
        arena.trim_end(),
        oracle.trim_end(),
        "arena ast-dump of the frontend's own source diverged from the oracle"
    );
}

#[test]
#[ignore = "slow capstone gate; run with `cargo test --release -- --ignored`"]
fn fixpoint_resolve_of_self_is_clean_like_oracle() {
    let module = zeta::parse_source(FRONTEND_SOURCE).expect("oracle parse of self should succeed");
    assert!(
        zeta::resolver::resolve(&module).is_ok(),
        "oracle resolver should find the frontend's own source clean"
    );
    let report = run_entry("resolve_report", FRONTEND_SOURCE);
    assert_eq!(
        report.trim_end(),
        "",
        "Zeta resolver reported diagnostics on the frontend's own (clean) source"
    );
}

#[test]
#[ignore = "slow capstone gate; run with `cargo test --release -- --ignored`"]
fn fixpoint_typecheck_of_self_is_clean_like_oracle() {
    let module = zeta::parse_source(FRONTEND_SOURCE).expect("oracle parse of self should succeed");
    assert!(
        zeta::typecheck::check(&module).is_ok(),
        "oracle typechecker should find the frontend's own source clean"
    );
    let report = run_entry("typecheck_report", FRONTEND_SOURCE);
    assert_eq!(
        report.trim_end(),
        "",
        "Zeta typechecker reported diagnostics on the frontend's own (clean) source"
    );
}

#[test]
#[ignore = "slow capstone gate; run with `cargo test --release -- --ignored`"]
fn fixpoint_mir_dump_of_self_matches_oracle() {
    let oracle = zeta::dump_mir(FRONTEND_SOURCE).expect("oracle mir-dump of self should succeed");
    let arena = run_entry("mir_dump_pipeline_via_arena", FRONTEND_SOURCE);
    assert_eq!(
        arena.trim_end(),
        oracle.trim_end(),
        "arena mir-dump of the frontend's own source diverged from the oracle"
    );
}
