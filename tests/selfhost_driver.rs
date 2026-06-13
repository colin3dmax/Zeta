// M7 self-hosting milestone (slice 2): the unified driver gate. The Zeta
// frontend exposes a single `compile(source, mode)` front door
// (testdata/selfhost/arena_frontend.zeta) that dispatches to the per-stage
// entry points. These tests prove the dispatch routes each mode to the right
// stage by checking `compile(src, mode)` against the same Rust oracle the
// individual per-stage gates use:
//
//   * compile(src, "ast-dump")  == zeta::dump_ast(src)
//   * compile(src, "resolve")   == (empty for clean source)
//   * compile(src, "typecheck") == (empty for clean source)
//   * compile(src, "mir-dump")  == zeta::dump_mir(src)
//   * compile(src, "run")       == zeta::run_source(src).to_string()
//   * compile(src, "bogus")     == "ZETAC_UNKNOWN_MODE bogus"
//
// Unlike the slow whole-self fixpoint gate, these run over tiny programs and
// stay in the routine `cargo test` set.

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

/// Run the unified driver `compile(program_source, mode)` inside the Zeta
/// interpreter and return the string it produces.
fn drive(mode: &str, program_source: &str) -> String {
    let caller = format!(
        r#"
module selfhost.caller;
import selfhost.arena_frontend;

fn main() -> String {{
  let source: String = {literal};
  return selfhost.arena_frontend.compile(source, "{mode}");
}}
"#,
        literal = zeta_string_literal(program_source),
    );

    let value = zeta::module_graph::run_sources(&[
        source_file("testdata/selfhost/arena_frontend.zeta", FRONTEND_SOURCE),
        source_file("testdata/selfhost/caller.zeta", &caller),
    ])
    .expect("unified driver caller should run");

    value.to_string()
}

#[test]
fn driver_ast_dump_routes_to_oracle() {
    let src = "fn main() -> Int { let x: Int = 1; return x + 2; }";
    let oracle = zeta::dump_ast(src).expect("oracle ast-dump should succeed");
    assert_eq!(
        drive("ast-dump", src).trim_end(),
        oracle.trim_end(),
        "compile(.., \"ast-dump\") did not route to the ast-dump stage"
    );
}

#[test]
fn driver_resolve_routes_and_is_clean() {
    let src = "fn main() -> Int { let x: Int = 1; return x; }";
    assert!(
        zeta::resolver::resolve(&zeta::parse_source(src).unwrap()).is_ok(),
        "oracle resolver should find this clean"
    );
    assert_eq!(
        drive("resolve", src).trim_end(),
        "",
        "compile(.., \"resolve\") should report nothing on clean source"
    );
}

#[test]
fn driver_typecheck_routes_and_is_clean() {
    let src = "fn main() -> Int { let x: Int = 1; return x; }";
    assert!(
        zeta::typecheck::check(&zeta::parse_source(src).unwrap()).is_ok(),
        "oracle typechecker should find this clean"
    );
    assert_eq!(
        drive("typecheck", src).trim_end(),
        "",
        "compile(.., \"typecheck\") should report nothing on clean source"
    );
}

#[test]
fn driver_mir_dump_routes_to_oracle() {
    let src = "fn main() -> Int { let x: Int = 1; return x + 2; }";
    let oracle = zeta::dump_mir(src).expect("oracle mir-dump should succeed");
    assert_eq!(
        drive("mir-dump", src).trim_end(),
        oracle.trim_end(),
        "compile(.., \"mir-dump\") did not route to the mir-dump stage"
    );
}

#[test]
fn driver_run_routes_to_oracle() {
    let src = "fn main() -> Int { let x: Int = 40; return x + 2; }";
    let oracle = zeta::run_source(src).expect("oracle run should succeed").to_string();
    assert_eq!(
        drive("run", src).trim_end(),
        oracle.trim_end(),
        "compile(.., \"run\") did not route to the interpreter stage"
    );
}

#[test]
fn driver_unknown_mode_returns_marker() {
    let src = "fn main() -> Int { return 0; }";
    assert_eq!(
        drive("bogus", src).trim_end(),
        "ZETAC_UNKNOWN_MODE bogus",
        "unknown mode should return the marker instead of crashing"
    );
}
