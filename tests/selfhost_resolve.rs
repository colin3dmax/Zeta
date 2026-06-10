// M3 self-hosting vertical slice #1: a minimal name resolver written in Zeta
// (testdata/selfhost/arena_frontend.zeta, `resolve_report`) consumes the arena
// AST and reports every "unknown name reference" (RESOLVE_UNKNOWN_NAME). Its
// report text must match the Rust resolver's RESOLVE_UNKNOWN_NAME diagnostics,
// in AST-visit order.
//
// Each case runs a tiny Zeta caller app that imports the frontend module and
// calls `resolve_report(<source>)`, then asserts the returned string equals the
// oracle report derived from `zeta::resolver::resolve`.

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

/// Run the Zeta resolver over `program_source` inside the interpreter and return
/// the report string it produces.
fn zeta_resolve_report(program_source: &str) -> String {
    let caller = format!(
        r#"
module selfhost.caller;
import selfhost.arena_frontend;

fn main() -> String {{
  let source: String = {literal};
  return selfhost.arena_frontend.resolve_report(source);
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
    .expect("resolver caller should run");

    value.to_string()
}

/// Oracle: run the Rust resolver, keep only RESOLVE_UNKNOWN_NAME diagnostics in
/// the order they were emitted, extract the name from each message
/// (`unknown name `X` in ...`), and format them identically to the Zeta report.
fn oracle_report(program_source: &str) -> String {
    let module = zeta::parse_source(program_source).expect("oracle parse should succeed");
    let mut out = String::new();
    if let Err(diagnostics) = zeta::resolver::resolve(&module) {
        for diagnostic in diagnostics {
            if diagnostic.code != "RESOLVE_UNKNOWN_NAME" {
                continue;
            }
            let name = extract_unknown_name(&diagnostic.message)
                .expect("RESOLVE_UNKNOWN_NAME message should contain a backtick-quoted name");
            out.push_str("RESOLVE_UNKNOWN_NAME name=");
            out.push_str(name);
            out.push('\n');
        }
    }
    out
}

/// Pull `X` out of a message of the form: unknown name `X` in function `fn`.
fn extract_unknown_name(message: &str) -> Option<&str> {
    let start = message.find('`')? + 1;
    let rest = &message[start..];
    let end = rest.find('`')?;
    Some(&rest[..end])
}

fn assert_matches_oracle(program_source: &str) {
    let oracle = oracle_report(program_source);
    let zeta = zeta_resolve_report(program_source);
    assert_eq!(
        zeta.trim_end(),
        oracle.trim_end(),
        "\n--- source ---\n{program_source}\n--- zeta ---\n{zeta}\n--- oracle ---\n{oracle}\n"
    );
}

#[test]
fn resolve_all_defined_reports_nothing() {
    // params + lets + top-level fn mutual references are all in scope.
    assert_matches_oracle(
        "fn helper(x: Int) -> Int { return x; } fn main(a: Int) -> Int { let b: Int = a + 1; let c: Int = helper(b); return b + c; }",
    );
}

#[test]
fn resolve_undefined_variable() {
    assert_matches_oracle("fn f() -> Int { return missing; }");
}

#[test]
fn resolve_let_is_sequentially_scoped() {
    // `y` is referenced before it is declared -> unknown.
    assert_matches_oracle("fn f() -> Int { let x: Int = y; let y: Int = 1; return x; }");
}

#[test]
fn resolve_let_cannot_see_itself() {
    assert_matches_oracle("fn f() -> Int { let z: Int = z; return 0; }");
}

#[test]
fn resolve_if_block_scope_does_not_leak() {
    // `inner` declared inside the then-block is unknown after the block.
    assert_matches_oracle(
        "fn f(c: Bool) -> Int { if c { let inner: Int = 1; } return inner; }",
    );
}

#[test]
fn resolve_while_block_scope_does_not_leak() {
    assert_matches_oracle(
        "fn f(c: Bool) -> Int { while c { let w: Int = 1; } return w; }",
    );
}

#[test]
fn resolve_for_binding_visible_in_body_not_outside() {
    // `i` is visible in the body (no report there) but unknown after the loop.
    assert_matches_oracle(
        "fn f() -> Int { for i in 0..n { let s: Int = i; } return i; }",
    );
}

#[test]
fn resolve_for_binding_used_in_body_ok() {
    assert_matches_oracle("fn f(xs: IntArray) -> Int { for x in xs { let y: Int = x; } return 0; }");
}

#[test]
fn resolve_match_arm_binding_visible_in_arm() {
    assert_matches_oracle(
        "fn f(x: Int) -> Int { match x { n -> { return n; } } }",
    );
}

#[test]
fn resolve_match_variant_payload_binding_visible_in_arm() {
    assert_matches_oracle(
        "fn f(s: Shape) -> Int { match s { Shape.Box(b) -> { return b; }, _ -> { return 0; } } }",
    );
}

#[test]
fn resolve_match_arm_binding_does_not_leak() {
    assert_matches_oracle(
        "fn f(x: Int) -> Int { match x { n -> { return 0; } } return n; }",
    );
}

#[test]
fn resolve_multiple_unknowns_in_order() {
    assert_matches_oracle("fn f() -> Int { return a + b + c; }");
}

#[test]
fn resolve_unknown_assign_target() {
    assert_matches_oracle("fn f() -> Int { undeclared = 1; return 0; }");
}

#[test]
fn resolve_complex_assign_target_root() {
    // `obj` (the field-chain root) is unknown; `field` is not a name reference.
    assert_matches_oracle("fn f() -> Int { obj.field = 1; return 0; }");
}

#[test]
fn resolve_struct_literal_type_and_fields_not_unknown() {
    // `Point` (type) and `x`/`y` (field names) must NOT be reported; only the
    // value expression `bad` is unknown.
    assert_matches_oracle(
        "fn f() -> Int { let p: Point = Point { x: 1, y: bad }; return 0; }",
    );
}

#[test]
fn resolve_qualified_call_path_not_unknown() {
    // A qualified call path is not a local name reference (it's a call callee).
    assert_matches_oracle("fn f() -> Int { return demo.util.compute(); }");
}

#[test]
fn resolve_field_access_field_name_not_unknown() {
    // base `rec` is in scope; field names `a`/`b` are not name references.
    assert_matches_oracle("fn f(rec: Thing) -> Int { let v: Int = rec.a.b; return 0; }");
}

#[test]
fn resolve_call_args_are_checked() {
    // The callee is not a name reference, but the argument `arg` is unknown.
    assert_matches_oracle("fn f() -> Int { return g(arg); }");
}

#[test]
fn resolve_nested_blocks_mixed() {
    assert_matches_oracle(
        "fn f(a: Bool, n: Int) -> Int { let mut total: Int = 0; if a { for i in 0..n { total = total + i + extra; } } return total + leftover; }",
    );
}

#[test]
fn resolve_for_c_scope() {
    // `i` from the C-for init is visible in condition/step/body, unknown after.
    assert_matches_oracle(
        "fn f(n: Int) -> Int { for (let mut i: Int = 0; i < n; i = i + 1) { let s: Int = i; } return i; }",
    );
}

#[test]
fn resolve_matches_oracle_on_nested_scope_kitchen_sink() {
    // c(let init), e(in-if), then d/i/g all out-of-block at the tail → 5 unknowns in order.
    assert_matches_oracle(
        "fn f(a: Int) -> Int { let b: Int = a + c; if a < b { let d: Int = a; return d + e; } for i in 0..b { let g: Int = i; } return d + i + g; }",
    );
}
