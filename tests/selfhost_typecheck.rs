// M4 self-hosting vertical slice #1: declared-type validation written in Zeta
// (testdata/selfhost/arena_frontend.zeta, `typecheck_report`) consumes the
// arena AST and reports TYPE_UNKNOWN_TYPE for every declared type name that is
// neither a builtin, a user struct/enum (whole-module prescan, so forward
// references are legal), nor a std-import enum. Its report text must match the
// Rust typechecker's TYPE_UNKNOWN_TYPE diagnostics in emit order
// (validate_declared_types runs before any function-body checking: items in
// source order, struct fields / enum payloads / fn params -> return type ->
// body statements recursively, with ForC visiting init -> step -> body).
//
// Each case runs a tiny Zeta caller app that imports the frontend module and
// calls `typecheck_report(<source>)`, then asserts the returned string equals
// the oracle report derived from `zeta::typecheck::check`.

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

/// Run the Zeta typechecker over `program_source` inside the interpreter and
/// return the report string it produces.
fn zeta_typecheck_report(program_source: &str) -> String {
    let caller = format!(
        r#"
module selfhost.caller;
import selfhost.arena_frontend;

fn main() -> String {{
  let source: String = {literal};
  return selfhost.arena_frontend.typecheck_report(source);
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
    .expect("typecheck caller should run");

    value.to_string()
}

/// Oracle: run the Rust typechecker, keep the diagnostics this slice
/// reproduces in the exact order they were emitted, and format them
/// identically to the Zeta report. Three shapes:
///   * TYPE_UNKNOWN_TYPE (slice 1): `... name=N span=S..E`
///   * expect-style codes (slice 2): message "expected {E}, found {F}" becomes
///     `CODE expected=E found=F span=S..E`
///   * fixed-message codes (slice 2): just `CODE span=S..E`
/// Any other code is filtered out (slice 3 territory the corpus avoids).
fn oracle_report(program_source: &str) -> String {
    let module = zeta::parse_source(program_source).expect("oracle parse should succeed");
    let mut out = String::new();
    if let Err(diagnostics) = zeta::typecheck::check(&module) {
        for diagnostic in diagnostics {
            match diagnostic.code {
                "TYPE_UNKNOWN_TYPE" => {
                    let name = extract_backtick_name(&diagnostic.message)
                        .expect("typecheck message should contain a backtick-quoted name");
                    out.push_str(&format!(
                        "TYPE_UNKNOWN_TYPE name={name} span={}..{}\n",
                        diagnostic.span.start, diagnostic.span.end
                    ));
                }
                "TYPE_LET_MISMATCH" | "TYPE_IF_CONDITION" | "TYPE_WHILE_CONDITION"
                | "TYPE_FORC_CONDITION" | "TYPE_RETURN_MISMATCH" | "TYPE_BINARY_OPERAND"
                | "TYPE_LOGICAL_OPERAND" | "TYPE_EQUALITY_OPERAND" | "TYPE_ORDERING_OPERAND"
                | "TYPE_UNARY_OPERAND" | "TYPE_RANGE_BOUND" => {
                    let rest = diagnostic
                        .message
                        .strip_prefix("expected ")
                        .expect("expect-style message should start with `expected `");
                    let (expected, found) = rest
                        .split_once(", found ")
                        .expect("expect-style message should contain `, found `");
                    out.push_str(&format!(
                        "{} expected={expected} found={found} span={}..{}\n",
                        diagnostic.code, diagnostic.span.start, diagnostic.span.end
                    ));
                }
                "TYPE_FOR_ITERABLE" | "TYPE_BREAK_OUTSIDE_LOOP" | "TYPE_CONTINUE_OUTSIDE_LOOP" => {
                    out.push_str(&format!(
                        "{} span={}..{}\n",
                        diagnostic.code, diagnostic.span.start, diagnostic.span.end
                    ));
                }
                _ => continue,
            }
        }
    }
    out
}

/// Pull `X` out of the first backtick pair in a message, e.g.
///   unknown type `X`
fn extract_backtick_name(message: &str) -> Option<&str> {
    let start = message.find('`')? + 1;
    let rest = &message[start..];
    let end = rest.find('`')?;
    Some(&rest[..end])
}

fn assert_matches_oracle(program_source: &str) {
    let oracle = oracle_report(program_source);
    let zeta = zeta_typecheck_report(program_source);
    assert_eq!(
        zeta.trim_end(),
        oracle.trim_end(),
        "\n--- source ---\n{program_source}\n--- zeta ---\n{zeta}\n--- oracle ---\n{oracle}\n"
    );
}

#[test]
fn typecheck_unknown_param_type() {
    assert_matches_oracle("fn f(x: Foo) -> Int { return 0; }");
}

#[test]
fn typecheck_unknown_return_type() {
    assert_matches_oracle("fn f() -> Bar { return 0; }");
}

#[test]
fn typecheck_unknown_let_type() {
    assert_matches_oracle("fn f() -> Int { let x: Baz = 1; return 0; }");
}

#[test]
fn typecheck_untyped_let_not_validated() {
    // An untyped `let` declares no type and must not report.
    assert_matches_oracle("fn f() -> Int { let x = 1; return x; }");
}

#[test]
fn typecheck_unknown_struct_field_type() {
    assert_matches_oracle("struct S { a: Mystery, b: Int } fn f() -> Int { return 0; }");
}

#[test]
fn typecheck_unknown_enum_payload_type() {
    // Only the payload-carrying variant `A(Ghost)` declares a type.
    assert_matches_oracle("enum E { A(Ghost), B } fn f() -> Int { return 0; }");
}

#[test]
fn typecheck_builtins_all_known() {
    assert_matches_oracle(
        "fn f(a: Int, b: String, c: Bool, d: IntArray, e: StringArray, g: BoolArray) -> Int { return 0; }",
    );
}

#[test]
fn typecheck_forward_struct_reference_known() {
    // Struct/enum names are prescanned from the whole module — a parameter may
    // reference a struct declared later in the source.
    assert_matches_oracle("fn f(p: Point) -> Int { return 0; } struct Point { x: Int }");
}

#[test]
fn typecheck_user_enum_known() {
    assert_matches_oracle("enum Color { Red } fn f(c: Color) -> Int { return 0; }");
}

#[test]
fn typecheck_std_enum_type_known_with_import() {
    assert_matches_oracle("import std.core; fn f(o: OptionInt) -> Int { return 0; }");
}

#[test]
fn typecheck_std_enum_type_unknown_without_import() {
    assert_matches_oracle("fn f(o: OptionInt) -> Int { return 0; }");
}

#[test]
fn typecheck_std_io_enum_gated_separately() {
    // ResultString comes from std.io — importing std.core does not grant it.
    assert_matches_oracle("import std.core; fn f(r: ResultString) -> Int { return 0; }");
}

#[test]
fn typecheck_nested_blocks_all_validated() {
    // 7 unknowns, in the oracle's visit order: if-then (A1), if-else (A2),
    // while body (A3), for-in body (A4), C-for init (A5) before its body (A6),
    // match arm body (A7).
    assert_matches_oracle(
        "fn f(c: Bool) -> Int { if c { let a: A1 = 1; } else { let b: A2 = 2; } while c { let w: A3 = 3; } for i in 0..3 { let g: A4 = 4; } for (let mut k: A5 = 0; k < 1; k = k + 1) { let m: A6 = 5; } match c { _ -> { let n: A7 = 6; } } return 0; }",
    );
}

#[test]
fn typecheck_multiple_unknowns_in_item_order() {
    // Struct field (T1), then the fn's param (T2), return type (T3), and typed
    // let (T4) — items in source order, each item's declared types in order.
    assert_matches_oracle("struct S { a: T1 } fn f(x: T2) -> T3 { let y: T4 = 1; return 0; }");
}

// ---------------------------------------------------------------------------
// M4 slice #2: expression type inference. The corpus avoids the slice-3 gaps:
// function calls, index/field access, array/struct literals, match patterns,
// parenthesized reported expressions, and compound assignment.
// ---------------------------------------------------------------------------

#[test]
fn typecheck_let_mismatch_string_to_int() {
    // Span = the string-literal token, quotes included.
    assert_matches_oracle("fn f() -> Int { let x: Int = \"a\"; return x; }");
}

#[test]
fn typecheck_let_ok_no_report() {
    assert_matches_oracle("fn f() -> Int { let x: Int = 1 + 2; return x; }");
}

#[test]
fn typecheck_untyped_let_propagates() {
    // The untyped let binds the inferred String, so the return mismatches.
    assert_matches_oracle("fn f() -> Int { let x = \"a\"; return x; }");
}

#[test]
fn typecheck_let_mismatch_binds_declared() {
    // Only the LET_MISMATCH reports: x carries the DECLARED Int afterwards,
    // so `return x` is fine.
    assert_matches_oracle("fn f() -> Int { let x: Int = true; return x; }");
}

#[test]
fn typecheck_if_condition_int() {
    assert_matches_oracle("fn f() -> Int { if 1 { let a: Int = 0; } return 0; }");
}

#[test]
fn typecheck_while_condition_string() {
    assert_matches_oracle("fn f() -> Int { while \"s\" { let a: Int = 0; } return 0; }");
}

#[test]
fn typecheck_forc_condition_int() {
    assert_matches_oracle(
        "fn f() -> Int { for (let mut i: Int = 0; i + 1; i = i + 1) { let a: Int = 0; } return 0; }",
    );
}

#[test]
fn typecheck_binary_operand_right() {
    // Only the right operand reports (span = the string literal); the Binary
    // still yields Int, so the return itself is fine.
    assert_matches_oracle("fn f() -> Int { return 1 + \"a\"; }");
}

#[test]
fn typecheck_logical_operand() {
    assert_matches_oracle("fn f() -> Bool { return true && 1; }");
}

#[test]
fn typecheck_equality_operand() {
    // Equality expects the RIGHT side to match the left: expected=Int
    // found=String, span = the right operand.
    assert_matches_oracle("fn f() -> Bool { return 1 == \"a\"; }");
}

#[test]
fn typecheck_ordering_operand_left() {
    assert_matches_oracle("fn f() -> Bool { return \"a\" < 1; }");
}

#[test]
fn typecheck_unary_not_int() {
    assert_matches_oracle("fn f() -> Bool { return !1; }");
}

#[test]
fn typecheck_unary_neg_bool() {
    assert_matches_oracle("fn f() -> Int { return -true; }");
}

#[test]
fn typecheck_return_mismatch_bool() {
    assert_matches_oracle("fn f() -> Int { return true; }");
}

#[test]
fn typecheck_bare_return_unit_mismatch() {
    // `return;` is Unit against an Int return type; the oracle pins the span
    // at 0..0.
    assert_matches_oracle("fn f() -> Int { let x: Int = 1; return; }");
}

#[test]
fn typecheck_unit_fn_returning_int() {
    // No `->` clause means Unit: expected=Unit found=Int.
    assert_matches_oracle("fn f() { return 0; }");
}

#[test]
fn typecheck_break_continue_outside_loop() {
    // Two reports, each spanning its own keyword token.
    assert_matches_oracle("fn f() -> Int { break; continue; return 0; }");
}

#[test]
fn typecheck_break_inside_while_ok() {
    assert_matches_oracle("fn f(c: Bool) -> Int { while c { break; } return 0; }");
}

#[test]
fn typecheck_range_bound_bool() {
    // The start bound reports (expected=Int found=Bool); the Range still
    // yields Int elements, so the body let is fine.
    assert_matches_oracle("fn f() -> Int { for i in true..3 { let a: Int = i; } return 0; }");
}

#[test]
fn typecheck_for_iterable_int() {
    // Int is not iterable: fixed-message TYPE_FOR_ITERABLE on the iterable.
    assert_matches_oracle("fn f() -> Int { for x in 1 { let a: Int = 0; } return 0; }");
}

#[test]
fn typecheck_error_suppression_chain() {
    // `miss` is unknown -> "<error>" -> the Binary, the let, and the return
    // all suppress: zero diagnostics from either side.
    assert_matches_oracle("fn f() -> Int { let x: Int = miss + 1; return x; }");
}

#[test]
fn typecheck_shadowed_type_reverse_lookup() {
    // The inner `let x: Bool` shadows the outer Int binding, so `let y: Int =
    // x` reports expected=Int found=Bool; the outer `return x` stays Int.
    assert_matches_oracle(
        "fn f(c: Bool) -> Int { let x: Int = 1; if c { let x: Bool = true; let y: Int = x; } return x; }",
    );
}

#[test]
fn typecheck_unknown_type_then_infer_order() {
    // validate_declared_types runs first (TYPE_UNKNOWN_TYPE for Ghost), then
    // inference reports the b mismatch and the return mismatch in body order.
    // `a` binds "<error>" so its own let suppresses.
    assert_matches_oracle(
        "fn f() -> Int { let a: Ghost = 1; let b: Bool = 2; return b; }",
    );
}
