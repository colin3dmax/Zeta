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

// --- Batch 2: full expression spectrum + if/while/assign ---

#[test]
fn arena_matches_oracle_on_full_precedence_ladder() {
    assert_matches_oracle(
        "fn f() -> Bool { let r: Bool = a || b && c == d + e * f; return r; }",
    );
}

#[test]
fn arena_matches_oracle_on_bool_literals() {
    assert_matches_oracle(
        "fn f() -> Bool { let t: Bool = true; let g: Bool = false; return t; }",
    );
}

#[test]
fn arena_matches_oracle_on_unary_mix() {
    assert_matches_oracle("fn f() -> Bool { let r: Bool = !a && -b; return r; }");
}

#[test]
fn arena_matches_oracle_on_unary_bit_not() {
    assert_matches_oracle("fn f() -> Int { let r: Int = ~a + -b; return r; }");
}

#[test]
fn arena_matches_oracle_on_bitwise_chain() {
    assert_matches_oracle("fn f() -> Int { let r: Int = a & b | c ^ d; return r; }");
}

#[test]
fn arena_matches_oracle_on_modulo() {
    assert_matches_oracle("fn f() -> Int { let r: Int = a % b + c * d % e; return r; }");
}

#[test]
fn arena_matches_oracle_on_all_comparisons() {
    assert_matches_oracle(
        "fn f() -> Bool { let a: Bool = p == q; let b: Bool = p != q; let c: Bool = p < q; let d: Bool = p <= q; let e: Bool = p > q; let g: Bool = p >= q; return a; }",
    );
}

#[test]
fn arena_matches_oracle_on_if_only() {
    assert_matches_oracle("fn f() -> Int { if a { return 1; } return 0; }");
}

#[test]
fn arena_matches_oracle_on_if_else() {
    assert_matches_oracle("fn f() -> Int { if a { return 1; } else { return 2; } }");
}

#[test]
fn arena_matches_oracle_on_if_else_if_else_chain() {
    assert_matches_oracle(
        "fn f() -> Int { if a { return 1; } else if b { return 2; } else if c { return 3; } else { return 4; } }",
    );
}

#[test]
fn arena_matches_oracle_on_empty_else() {
    assert_matches_oracle("fn f() -> Int { if a { return 1; } return 0; }");
}

#[test]
fn arena_matches_oracle_on_while() {
    assert_matches_oracle(
        "fn f() -> Int { let mut i: Int = 0; while i < n { i = i + 1; } return i; }",
    );
}

#[test]
fn arena_matches_oracle_on_while_with_nested_if() {
    assert_matches_oracle(
        "fn f() -> Int { while c { x = x + 1; if d { x = x - 1; } } return x; }",
    );
}

#[test]
fn arena_matches_oracle_on_simple_assign() {
    assert_matches_oracle("fn f() -> Int { x = a + b * c; return x; }");
}

#[test]
fn arena_matches_oracle_on_params_and_mut() {
    assert_matches_oracle(
        "fn f(a: Int, b: Int) -> Int { let mut s: Int = a + b; s = s * 2; return s; }",
    );
}

#[test]
fn arena_matches_oracle_on_while_inside_if_else() {
    assert_matches_oracle(
        "fn f(a: Bool, n: Int) -> Int { let mut i: Int = 0; if a { while i < n { i = i + 1; } } else { while i > 0 { i = i - 1; } } return i; }",
    );
}

// --- Regression: existing stage1 parity probes that use only this batch's
// constructs. Probes relying on calls/indexing/field-access/struct/array
// literals/strings/break/continue/for are intentionally skipped (see the test
// returned summary for the list).
fn assert_probe(path: &str) {
    let source = std::fs::read_to_string(path).expect("probe source should read");
    assert_matches_oracle(&source);
}

#[test]
fn arena_matches_oracle_on_operator_probes() {
    // op_07 (parens), op_13 (call/index/field — skip), op_chain (call/string — skip).
    for name in [
        "op_01", "op_02", "op_03", "op_04", "op_05", "op_06", "op_07", "op_08", "op_09",
        "op_10", "op_11", "op_12", "op_14", "op_15",
    ] {
        assert_probe(&format!("testdata/stage1_parity/{name}.zeta"));
    }
}

#[test]
fn arena_matches_oracle_on_bitwise_neg_mod_probes() {
    for name in ["bitwise_01", "neg_01", "mod_01"] {
        assert_probe(&format!("testdata/stage1_parity/{name}.zeta"));
    }
}

#[test]
fn arena_matches_oracle_on_control_flow_probes() {
    // cf_05/cf_11/cf_12/cf_15 use break/continue and are skipped (not in this
    // batch); for_* are skipped too.
    for name in [
        "cf_01", "cf_02", "cf_03", "cf_04", "cf_06", "cf_07", "cf_08", "cf_09", "cf_10",
        "cf_13", "cf_14", "elif_01",
    ] {
        assert_probe(&format!("testdata/stage1_parity/{name}.zeta"));
    }
}

#[test]
fn arena_matches_oracle_on_review_kitchen_sink() {
    assert_matches_oracle(
        "fn f(a: Int, b: Int) -> Bool { let mut r: Int = a || b && a == b + a * b; if a < b { r = a; } else if a > b { r = b; } else { r = 0; } while r < a & b | a { r = r + 1; } return !r == false && -a < ~b; }",
    );
}
