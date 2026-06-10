// M2 self-hosting vertical slice: an arena-based recursive-descent frontend
// written in Zeta (testdata/selfhost/arena_frontend.zeta) must produce dump
// text byte-for-byte identical to the Rust `ast-dump` oracle.
//
// Each case runs a tiny Zeta caller app that imports the frontend module and
// calls `dump_module_via_arena(<source>)`, then asserts the returned string
// equals `zeta::dump_ast(<source>)` (trimmed of trailing whitespace).

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

/// Run the arena frontend over `program_source` inside the Zeta interpreter and
/// return the dump string it produces.
fn arena_dump(program_source: &str) -> String {
    let caller = format!(
        r#"
module selfhost.caller;
import selfhost.arena_frontend;

fn main() -> String {{
  let source: String = {literal};
  return selfhost.arena_frontend.dump_module_via_arena(source);
}}
"#,
        literal = zeta_string_literal(program_source)
    );

    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/selfhost/arena_frontend.zeta",
            include_str!("../testdata/selfhost/arena_frontend.zeta"),
        ),
        source_file("testdata/selfhost/caller.zeta", &caller),
    ])
    .expect("arena frontend caller should run");

    value.to_string()
}

fn assert_matches_oracle(program_source: &str) {
    let oracle = zeta::dump_ast(program_source).expect("Rust ast-dump oracle should succeed");
    let arena = arena_dump(program_source);
    assert_eq!(
        arena.trim_end(),
        oracle.trim_end(),
        "\n--- source ---\n{program_source}\n--- arena ---\n{arena}\n--- oracle ---\n{oracle}\n"
    );
}

#[test]
fn arena_matches_oracle_on_arithmetic_precedence() {
    assert_matches_oracle(
        "module demo.app; fn main() -> Int { let x: Int = 1 + 2 * 3; return x; }",
    );
}

#[test]
fn arena_matches_oracle_on_parenthesized_expression() {
    assert_matches_oracle("fn f() -> Int { let y: Int = (1 + 2) * 3; return y; }");
}

#[test]
fn arena_matches_oracle_on_left_associative_chain() {
    assert_matches_oracle("fn f() -> Int { let z: Int = 5 - 2 - 1; return z; }");
}

#[test]
fn arena_matches_oracle_on_multiple_lets_and_name_expr() {
    assert_matches_oracle(
        "module m; fn g() -> Int { let a: Int = 10; let b: Int = a + 4 / 2; return b; }",
    );
}

#[test]
fn arena_matches_oracle_on_multiple_functions() {
    assert_matches_oracle(
        "fn a() -> Int { return 1; } fn b() -> Int { let x: Int = 5 - 2 - 1; return x; }",
    );
}

#[test]
fn arena_matches_oracle_on_name_only_return() {
    assert_matches_oracle("module solo.app; fn only() -> Bool { return foo; }");
}

#[test]
fn arena_matches_oracle_on_deep_mixed_precedence() {
    assert_matches_oracle(
        "module deep.test; fn compute() -> Int { let a: Int = 1 + 2 * 3 - 4 / 2; let b: Int = (a + 1) * (a - 2) / (a + 3); let c: Int = a + b * a - b; return c; } fn second() -> Int { let z: Int = ((1 + 2) * (3 + 4)) - 5; return z; }",
    );
}
