// M3 self-hosting vertical slices: a minimal name resolver written in Zeta
// (testdata/selfhost/arena_frontend.zeta, `resolve_report`) consumes the arena
// AST and reports resolver diagnostics — unknown name references
// (RESOLVE_UNKNOWN_NAME), unknown function calls (RESOLVE_UNKNOWN_FUNCTION),
// duplicate local definitions (RESOLVE_DUPLICATE_LOCAL), and duplicate
// top-level items (RESOLVE_DUPLICATE_ITEM). Its report text must match the
// Rust resolver's diagnostics for those codes, in emit order (top-level item
// diagnostics are emitted before any function-body diagnostics, mirroring
// check_top_level running first).
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

/// Oracle: run the Rust resolver, keep the three resolver diagnostics this slice
/// reproduces (RESOLVE_UNKNOWN_NAME, RESOLVE_UNKNOWN_FUNCTION,
/// RESOLVE_DUPLICATE_LOCAL) in the exact order they were emitted, extract the
/// backtick-quoted name/callee from each message, and format them identically to
/// the Zeta report.
fn oracle_report(program_source: &str) -> String {
    let module = zeta::parse_source(program_source).expect("oracle parse should succeed");
    let mut out = String::new();
    if let Err(diagnostics) = zeta::resolver::resolve(&module) {
        for diagnostic in diagnostics {
            let label = match diagnostic.code {
                "RESOLVE_UNKNOWN_NAME" => "RESOLVE_UNKNOWN_NAME name=",
                "RESOLVE_UNKNOWN_FUNCTION" => "RESOLVE_UNKNOWN_FUNCTION name=",
                "RESOLVE_DUPLICATE_LOCAL" => "RESOLVE_DUPLICATE_LOCAL name=",
                "RESOLVE_DUPLICATE_ITEM" => "RESOLVE_DUPLICATE_ITEM name=",
                "RESOLVE_UNKNOWN_IMPORT" => "RESOLVE_UNKNOWN_IMPORT name=",
                _ => continue,
            };
            let name = extract_backtick_name(&diagnostic.message)
                .expect("resolver message should contain a backtick-quoted name");
            out.push_str(label);
            out.push_str(name);
            out.push_str(&format!(
                " span={}..{}",
                diagnostic.span.start, diagnostic.span.end
            ));
            out.push('\n');
        }
    }
    out
}

/// Pull `X` out of the first backtick pair in a message, e.g.
///   unknown name `X` in function `fn`
///   unknown function `a.b.f` in function `fn`
///   duplicate local `X` in function `fn`
fn extract_backtick_name(message: &str) -> Option<&str> {
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

// --- slice #2: RESOLVE_UNKNOWN_FUNCTION ------------------------------------

#[test]
fn resolve_call_undefined_function_reported() {
    assert_matches_oracle("fn f() -> Int { return g(); }");
}

#[test]
fn resolve_call_defined_top_level_fn_ok() {
    // `g` is a top-level function → its call is not an unknown function.
    assert_matches_oracle("fn g() -> Int { return 0; } fn f() -> Int { return g(); }");
}

#[test]
fn resolve_call_qualified_path_is_unknown_function() {
    // A dotted callee that is not an enum-variant call → unknown function
    // (the path itself is reported, e.g. `demo.util.compute`).
    assert_matches_oracle("fn f() -> Int { return demo.util.compute(); }");
}

#[test]
fn resolve_call_local_variable_as_callee_is_unknown_function() {
    // A local used as a callee is still an unknown function (locals are not
    // consulted for the call set), so `g` is reported.
    assert_matches_oracle("fn f() -> Int { let g: Int = 0; return g(); }");
}

#[test]
fn resolve_enum_variant_call_is_not_unknown_function() {
    // `Sh.Box` is an enum-variant call → not reported as an unknown function.
    assert_matches_oracle(
        "enum Sh { Box, Circle } fn f() -> Int { let x: Sh = Sh.Box(); return 0; }",
    );
}

#[test]
fn resolve_unknown_function_then_args_order() {
    // Callee `g` is reported before its argument `arg` is checked.
    assert_matches_oracle("fn f() -> Int { return g(arg); }");
}

// --- slice #2: RESOLVE_DUPLICATE_LOCAL -------------------------------------

#[test]
fn resolve_duplicate_two_lets_same_block() {
    assert_matches_oracle("fn f() -> Int { let x: Int = 1; let x: Int = 2; return x; }");
}

#[test]
fn resolve_duplicate_param_and_let() {
    assert_matches_oracle("fn f(x: Int) -> Int { let x: Int = 1; return x; }");
}

#[test]
fn resolve_duplicate_two_params() {
    assert_matches_oracle("fn f(x: Int, x: Int) -> Int { return x; }");
}

#[test]
fn resolve_inner_block_let_shadowing_outer_let_is_duplicate() {
    // Counterintuitive: shadowing an outer `let` from a nested block reports a
    // duplicate (the Rust resolver clones the locals map into the block).
    assert_matches_oracle(
        "fn f(c: Bool) -> Int { let x: Int = 1; if c { let x: Int = 2; } return x; }",
    );
}

#[test]
fn resolve_sibling_blocks_same_let_name_not_duplicate() {
    // Two sibling blocks each declaring `x` are independent → no duplicate.
    assert_matches_oracle(
        "fn f(c: Bool) -> Int { if c { let x: Int = 1; } if c { let x: Int = 2; } return 0; }",
    );
}

#[test]
fn resolve_for_binding_then_let_same_name_is_duplicate() {
    // The for body's `let i` collides with the loop binding `i` → duplicate.
    assert_matches_oracle(
        "fn f() -> Int { for i in 0..3 { let i: Int = 1; } return 0; }",
    );
}

#[test]
fn resolve_for_binding_shadowing_outer_local_not_duplicate() {
    // A for binding shadowing an outer local is inserted unconditionally → no
    // duplicate (only a body `let` of the same name would collide).
    assert_matches_oracle(
        "fn f(i: Int) -> Int { for i in 0..3 { return i; } return 0; }",
    );
}

#[test]
fn resolve_match_binding_then_let_same_name_is_duplicate() {
    assert_matches_oracle(
        "fn f(v: Int) -> Int { match v { n -> { let n: Int = 1; } } return 0; }",
    );
}

#[test]
fn resolve_match_binding_shadowing_outer_local_not_duplicate() {
    assert_matches_oracle(
        "fn f(n: Int) -> Int { match n { n -> { return n; } } return 0; }",
    );
}

#[test]
fn resolve_for_c_init_shadowing_outer_local_is_duplicate() {
    // The C-for init is a `let`, so it shadowing an outer `i` reports a duplicate
    // (unlike a plain for-in binding).
    assert_matches_oracle(
        "fn f(i: Int) -> Int { for (let mut i: Int = 0; i < 3; i = i + 1) { return i; } return 0; }",
    );
}

// --- slice #3: RESOLVE_DUPLICATE_ITEM ---------------------------------------

#[test]
fn resolve_duplicate_top_level_fns() {
    assert_matches_oracle("fn f() -> Int { return 0; } fn f() -> Int { return 1; }");
}

#[test]
fn resolve_duplicate_struct_and_fn_same_name() {
    // Duplicate detection is purely name-based across item kinds.
    assert_matches_oracle("struct s { x: Int } fn s() -> Int { return 0; }");
}

#[test]
fn resolve_duplicate_enum_and_struct_same_name() {
    assert_matches_oracle("enum T { A, B } struct T { x: Int } fn f() -> Int { return 0; }");
}

#[test]
fn resolve_triple_duplicate_reports_twice() {
    // Every occurrence after the first reports (the second and third).
    assert_matches_oracle(
        "fn f() -> Int { return 0; } fn f() -> Int { return 1; } fn f() -> Int { return 2; }",
    );
}

#[test]
fn resolve_distinct_top_level_names_no_duplicate() {
    assert_matches_oracle(
        "struct A { x: Int } enum B { C } fn d() -> Int { return 0; }",
    );
}

#[test]
fn resolve_duplicate_item_reported_before_function_diagnostics() {
    // check_top_level runs before function bodies: the duplicate-item report for
    // `f` precedes the unknown-name report from the FIRST function's body even
    // though the body appears earlier in the source.
    assert_matches_oracle(
        "fn f() -> Int { return miss; } fn f() -> Int { return 0; }",
    );
}

#[test]
fn resolve_duplicate_fn_still_resolves_both_bodies() {
    // Both duplicate definitions' bodies are resolved (each reports its own
    // unknown name, after the duplicate-item line).
    assert_matches_oracle(
        "fn f() -> Int { return one; } fn f() -> Int { return two; }",
    );
}

// --- slice #4: import boundary (RESOLVE_UNKNOWN_IMPORT + std name sets) -----

#[test]
fn resolve_unknown_import_reported() {
    assert_matches_oracle("import foo.bar; fn f() -> Int { return 0; }");
}

#[test]
fn resolve_single_segment_unknown_import() {
    assert_matches_oracle("import foo; fn f() -> Int { return 0; }");
}

#[test]
fn resolve_std_prefix_but_unknown_import() {
    // `std.fake` is not a standard import — prefix matching must be exact.
    assert_matches_oracle("import std.fake; fn f() -> Int { return 0; }");
}

#[test]
fn resolve_std_core_import_ok_and_fn_known() {
    assert_matches_oracle(
        "import std.core; fn f() -> Int { return string_len(\"a\"); }",
    );
}

#[test]
fn resolve_std_core_fn_unknown_without_import() {
    assert_matches_oracle("fn f() -> Int { return string_len(\"a\"); }");
}

#[test]
fn resolve_std_io_import_ok_and_fn_known() {
    assert_matches_oracle(
        "import std.io; fn f() -> String { return path_join(\"a\", \"b\"); }",
    );
}

#[test]
fn resolve_std_core_does_not_grant_io_fns() {
    assert_matches_oracle(
        "import std.core; fn f() -> String { return path_join(\"a\", \"b\"); }",
    );
}

#[test]
fn resolve_std_enum_variant_call_known_with_import() {
    assert_matches_oracle(
        "import std.core; fn f() -> Int { let x: OptionInt = OptionInt.Some(1); return 0; }",
    );
}

#[test]
fn resolve_std_enum_bare_reference_known_with_import() {
    // Std enum names join top_level_names → a bare reference is not unknown.
    assert_matches_oracle("import std.core; fn f() -> Int { return OptionInt; }");
}

#[test]
fn resolve_std_enum_bare_reference_unknown_without_import() {
    assert_matches_oracle("fn f() -> Int { return OptionInt; }");
}

#[test]
fn resolve_std_fn_bare_reference_is_unknown_name() {
    // Std FUNCTION names are callable but are NOT legal bare references
    // (functions joins the callee set, not top_level_names).
    assert_matches_oracle("import std.core; fn f() -> Int { return string_len; }");
}

#[test]
fn resolve_item_conflicts_with_std_core_name() {
    assert_matches_oracle(
        "import std.core; enum OptionInt { A } fn f() -> Int { return 0; }",
    );
}

#[test]
fn resolve_item_conflicts_with_std_io_name() {
    assert_matches_oracle(
        "import std.io; struct ResultString { x: Int } fn f() -> Int { return 0; }",
    );
}

#[test]
fn resolve_item_before_import_still_conflicts() {
    // standard_top_level_names is computed from the whole module before the
    // item loop — an item preceding the import still conflicts.
    assert_matches_oracle(
        "enum ResultInt { A } import std.core; fn f() -> Int { return 0; }",
    );
}

#[test]
fn resolve_user_duplicate_and_std_conflict_both_report() {
    // Second `ResultInt` item reports a user-level duplicate AND a std
    // conflict; the first reports only the conflict. Three lines total.
    assert_matches_oracle(
        "import std.core; fn ResultInt() -> Int { return 0; } struct ResultInt { x: Int }",
    );
}

#[test]
fn resolve_no_conflict_without_the_import() {
    assert_matches_oracle("enum OptionInt { A } fn f() -> Int { return 0; }");
}

#[test]
fn resolve_unknown_imports_interleaved_with_duplicates_in_item_order() {
    assert_matches_oracle(
        "import a.b; fn f() -> Int { return 0; } import c.d; fn f() -> Int { return 1; }",
    );
}

#[test]
fn resolve_repeated_std_import_not_duplicate() {
    // Imports have no item name → never duplicate items.
    assert_matches_oracle(
        "import std.core; import std.core; fn f() -> Int { return 0; }",
    );
}

#[test]
fn resolve_std_import_with_alias_is_known() {
    assert_matches_oracle(
        "import std.core as core; fn f() -> Int { return string_len(\"a\"); }",
    );
}

// --- slice #2: all three codes mixed, order preserved ----------------------

#[test]
fn resolve_three_codes_mixed_in_order() {
    // Expected emit order:
    //   UNKNOWN_NAME miss      (first let initializer)
    //   UNKNOWN_FUNCTION g     (second let: callee checked before args)
    //   UNKNOWN_NAME arg       (call argument)
    //   DUPLICATE_LOCAL x      (second `let x` after its initializer resolves)
    assert_matches_oracle(
        "fn f() -> Int { let x: Int = miss; let x: Int = g(arg); return x; }",
    );
}

#[test]
fn resolve_matches_oracle_on_three_code_reversal_kitchen_sink() {
    // dup param x; unknown name; unknown fn; dup let a; for-in shadow (NOT dup);
    // C-for init shadow (DUP) — the for-in vs for-C asymmetry, all vs oracle.
    assert_matches_oracle(
        "fn helper() -> Int { return 0; } fn f(x: Int, x: Int) -> Int { let a: Int = ghostVar; let b: Int = ghostFn(); let a: Int = 2; for x in [1, 2] { let c: Int = helper(); } for (let mut x: Int = 0; x < 1; x = x + 1) { let d: Int = a; } return helper(); }",
    );
}

// --- slice #5: exact span alignment -----------------------------------------

#[test]
fn resolve_span_distinguishes_repeated_unknown_names() {
    // The two `x` references differ ONLY in their byte spans.
    assert_matches_oracle("fn f() -> Int { return x + x; }");
}

#[test]
fn resolve_span_multiline_source() {
    // Byte offsets count newlines like any other byte.
    assert_matches_oracle("fn f() -> Int {\n  let a: Int = 1;\n  return missing;\n}");
}

#[test]
fn resolve_span_qualified_callee_with_spaces() {
    // The callee span runs from the first path ident to the last, covering the
    // interior whitespace around the dots.
    assert_matches_oracle("fn f() -> Int { return demo . util . compute(); }");
}

#[test]
fn resolve_span_import_with_spaces() {
    // The import path span likewise covers `a . b` (not the keyword/semicolon).
    assert_matches_oracle("import a . b; fn f() -> Int { return 0; }");
}

#[test]
fn resolve_span_duplicate_local_on_second_binding() {
    // The duplicate-local span points at the SECOND (new) binding's ident.
    assert_matches_oracle("fn f() -> Int { let x: Int = 1; let x: Int = 2; return x; }");
}

#[test]
fn resolve_span_duplicate_item_on_second_item() {
    // The duplicate-item span points at the second item's name token.
    assert_matches_oracle("fn dup() -> Int { return 0; } fn dup() -> Int { return 1; }");
}
