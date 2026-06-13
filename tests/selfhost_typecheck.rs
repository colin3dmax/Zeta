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
/// identically to the Zeta report. Four shapes:
///   * name-carrying codes (slices 1 + 3): `CODE name=N span=S..E`
///   * expect-style codes (slices 2 + 3): message "expected {E}, found {F}"
///     becomes `CODE expected=E found=F span=S..E`
///   * fixed-message codes (slices 2 + 3): just `CODE span=S..E`
///   * TYPE_CALL_ARITY (slice 3): message "function `f` expects N arguments,
///     found M" becomes `TYPE_CALL_ARITY name=f expected=N found=M span=S..E`
///   * the slice-4 enum/match codes: TYPE_ENUM_VARIANT_ARITY (counted call
///     forms + kind=uncalled), TYPE_ENUM_PATTERN_ARITY (kind=unbound /
///     kind=spurious) and TYPE_MATCH_NON_EXHAUSTIVE (fixed for Bool, named
///     for enums) — see the match arms below for the message shapes.
fn oracle_report(program_source: &str) -> String {
    let module = zeta::parse_source(program_source).expect("oracle parse should succeed");
    let mut out = String::new();
    if let Err(diagnostics) = zeta::typecheck::check(&module) {
        for diagnostic in diagnostics {
            match diagnostic.code {
                "TYPE_UNKNOWN_TYPE"
                | "TYPE_UNKNOWN_STRUCT"
                | "TYPE_DUPLICATE_FIELD"
                | "TYPE_UNKNOWN_FIELD"
                | "TYPE_MISSING_FIELD"
                | "TYPE_ARRAY_FIELD"
                | "TYPE_ASSIGN_IMMUTABLE"
                | "TYPE_UNKNOWN_NAME"
                | "TYPE_UNKNOWN_VARIANT"
                | "TYPE_UNKNOWN_ENUM" => {
                    let name = extract_backtick_name(&diagnostic.message)
                        .expect("typecheck message should contain a backtick-quoted name");
                    out.push_str(&format!(
                        "{} name={name} span={}..{}\n",
                        diagnostic.code, diagnostic.span.start, diagnostic.span.end
                    ));
                }
                "TYPE_LET_MISMATCH"
                | "TYPE_IF_CONDITION"
                | "TYPE_WHILE_CONDITION"
                | "TYPE_FORC_CONDITION"
                | "TYPE_RETURN_MISMATCH"
                | "TYPE_BINARY_OPERAND"
                | "TYPE_LOGICAL_OPERAND"
                | "TYPE_EQUALITY_OPERAND"
                | "TYPE_ORDERING_OPERAND"
                | "TYPE_UNARY_OPERAND"
                | "TYPE_RANGE_BOUND"
                | "TYPE_CALL_ARGUMENT"
                | "TYPE_STRUCT_FIELD"
                | "TYPE_ARRAY_ELEMENT"
                | "TYPE_INDEX"
                | "TYPE_ASSIGN_MISMATCH"
                | "TYPE_ENUM_VARIANT_PAYLOAD"
                | "TYPE_MATCH_PATTERN" => {
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
                "TYPE_FOR_ITERABLE"
                | "TYPE_BREAK_OUTSIDE_LOOP"
                | "TYPE_CONTINUE_OUTSIDE_LOOP"
                | "TYPE_ARRAY_EMPTY"
                | "TYPE_INDEX_BASE"
                | "TYPE_FIELD_BASE"
                | "TYPE_ASSIGN_TARGET" => {
                    out.push_str(&format!(
                        "{} span={}..{}\n",
                        diagnostic.code, diagnostic.span.start, diagnostic.span.end
                    ));
                }
                "TYPE_ENUM_VARIANT_ARITY" => {
                    // Three message shapes:
                    //   "variant `E.V` expects 1 payload argument, found N"
                    //   "variant `E.V` expects no payload arguments, found N"
                    //   "variant `E.V` carries a payload and must be called"
                    let name = extract_backtick_name(&diagnostic.message)
                        .expect("variant arity message should contain a backtick-quoted name");
                    if diagnostic
                        .message
                        .contains("carries a payload and must be called")
                    {
                        out.push_str(&format!(
                            "TYPE_ENUM_VARIANT_ARITY name={name} kind=uncalled span={}..{}\n",
                            diagnostic.span.start, diagnostic.span.end
                        ));
                    } else if let Some((_, found)) = diagnostic
                        .message
                        .split_once("expects 1 payload argument, found ")
                    {
                        out.push_str(&format!(
                            "TYPE_ENUM_VARIANT_ARITY name={name} expected=1 found={found} span={}..{}\n",
                            diagnostic.span.start, diagnostic.span.end
                        ));
                    } else {
                        let (_, found) = diagnostic
                            .message
                            .split_once("expects no payload arguments, found ")
                            .expect("variant arity message should match a known shape");
                        out.push_str(&format!(
                            "TYPE_ENUM_VARIANT_ARITY name={name} expected=0 found={found} span={}..{}\n",
                            diagnostic.span.start, diagnostic.span.end
                        ));
                    }
                }
                "TYPE_ENUM_PATTERN_ARITY" => {
                    // Two message shapes:
                    //   "variant `E.V` carries a payload and must bind it"
                    //   "variant `E.V` does not carry a payload"
                    let name = extract_backtick_name(&diagnostic.message)
                        .expect("pattern arity message should contain a backtick-quoted name");
                    let kind = if diagnostic.message.contains("must bind it") {
                        "unbound"
                    } else {
                        assert!(
                            diagnostic.message.contains("does not carry a payload"),
                            "pattern arity message should match a known shape"
                        );
                        "spurious"
                    };
                    out.push_str(&format!(
                        "TYPE_ENUM_PATTERN_ARITY name={name} kind={kind} span={}..{}\n",
                        diagnostic.span.start, diagnostic.span.end
                    ));
                }
                "TYPE_MATCH_NON_EXHAUSTIVE" => {
                    // Bool matches use a fixed message; enum matches carry the
                    // enum name in backticks ("match on `E` must cover ...").
                    if diagnostic.message.starts_with("Bool match") {
                        out.push_str(&format!(
                            "TYPE_MATCH_NON_EXHAUSTIVE span={}..{}\n",
                            diagnostic.span.start, diagnostic.span.end
                        ));
                    } else {
                        let name = extract_backtick_name(&diagnostic.message).expect(
                            "enum non-exhaustive message should contain a backtick-quoted name",
                        );
                        out.push_str(&format!(
                            "TYPE_MATCH_NON_EXHAUSTIVE name={name} span={}..{}\n",
                            diagnostic.span.start, diagnostic.span.end
                        ));
                    }
                }
                "TYPE_CALL_ARITY" => {
                    let name = extract_backtick_name(&diagnostic.message)
                        .expect("arity message should contain a backtick-quoted name");
                    let rest = diagnostic
                        .message
                        .split_once("expects ")
                        .expect("arity message should contain `expects `")
                        .1;
                    let (expected, found) = rest
                        .split_once(" arguments, found ")
                        .expect("arity message should contain ` arguments, found `");
                    out.push_str(&format!(
                        "TYPE_CALL_ARITY name={name} expected={expected} found={found} span={}..{}\n",
                        diagnostic.span.start, diagnostic.span.end
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
// M4 slice #2: expression type inference. The corpus avoids the known quirks:
// struct names ending in "Array", struct literals missing more than one field
// (HashMap-random missing order), non-lvalue assignment targets,
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
    assert_matches_oracle("fn f() -> Int { let a: Ghost = 1; let b: Bool = 2; return b; }");
}

// ---------------------------------------------------------------------------
// M4 slice #3: Call / StructLiteral / ArrayLiteral / Index / FieldAccess
// inference plus Assign-target checking.
// ---------------------------------------------------------------------------

#[test]
fn typecheck_call_arity_zero_args() {
    // Arity reports at the callee path; the call still types as g's return.
    assert_matches_oracle("fn g(a: Int) -> Int { return a; } fn f() -> Int { return g(); }");
}

#[test]
fn typecheck_call_arity_extra_args() {
    // Early exit: the arity report returns immediately, arguments unvisited.
    assert_matches_oracle("fn g(a: Int) -> Int { return a; } fn f() -> Int { return g(1, 2); }");
}

#[test]
fn typecheck_call_argument_mismatch() {
    // expected=Int found=Bool, span = the argument expression.
    assert_matches_oracle("fn g(a: Int) -> Int { return a; } fn f() -> Int { return g(true); }");
}

#[test]
fn typecheck_call_return_type_used() {
    // The call types as g's return (String), so the let reports found=String.
    assert_matches_oracle(
        "fn g() -> String { return \"s\"; } fn f() -> Int { let x: Int = g(); return x; }",
    );
}

#[test]
fn typecheck_unit_fn_call_as_value() {
    // No `->` clause means the call yields Unit: found=Unit on the let.
    assert_matches_oracle("fn h() { } fn f() -> Int { let x: Int = h(); return x; }");
}

#[test]
fn typecheck_unknown_callee_silent_args_unvisited() {
    // An unknown callee is the resolver's business: the typechecker stays
    // silent AND never visits the arguments (the bad `true` cannot report),
    // and the "<error>" result suppresses the return check. Zero lines.
    assert_matches_oracle("fn f() -> Int { return ghost(true); }");
}

#[test]
fn typecheck_std_fn_signature() {
    // `import std.core` grants string_len(String) -> Int: the Int argument
    // reports TYPE_CALL_ARGUMENT expected=String found=Int; the return is Int
    // so the return check passes.
    assert_matches_oracle("import std.core; fn f() -> Int { return string_len(1); }");
}

#[test]
fn typecheck_std_fn_without_import_silent() {
    // Without the import string_len is just an unknown callee: zero lines.
    assert_matches_oracle("fn f() -> Int { return string_len(1); }");
}

#[test]
fn typecheck_struct_literal_ok() {
    assert_matches_oracle(
        "struct Point { x: Int, y: Int } fn f() -> Int { let p: Point = Point { x: 1, y: 2 }; return p.x; }",
    );
}

#[test]
fn typecheck_unknown_struct_literal() {
    // TYPE_UNKNOWN_TYPE for the let annotation comes first (declared-type
    // validation), then TYPE_UNKNOWN_STRUCT at the literal's type-name token.
    // The literal still types as Named(Ghost) but the let's declared type is
    // "<error>", so the let check suppresses.
    assert_matches_oracle("fn f() -> Int { let p: Ghost = Ghost { a: 1 }; return 0; }");
}

#[test]
fn typecheck_struct_field_mismatch() {
    // expected=Int found=Bool, span = the field's value expression.
    assert_matches_oracle(
        "struct Point { x: Int, y: Int } fn f() -> Int { let p: Point = Point { x: true, y: 2 }; return 0; }",
    );
}

#[test]
fn typecheck_struct_literal_unknown_field() {
    // `z` is not declared on Point: TYPE_UNKNOWN_FIELD at the field name.
    assert_matches_oracle(
        "struct Point { x: Int, y: Int } fn f() -> Int { let p: Point = Point { x: 1, y: 2, z: 3 }; return 0; }",
    );
}

#[test]
fn typecheck_struct_literal_duplicate_field() {
    // The SECOND `x` reports TYPE_DUPLICATE_FIELD at its own name token.
    assert_matches_oracle(
        "struct Point { x: Int, y: Int } fn f() -> Int { let p: Point = Point { x: 1, x: 2, y: 3 }; return 0; }",
    );
}

#[test]
fn typecheck_struct_literal_missing_one_field() {
    // Exactly ONE missing field (`y`) — Rust iterates missing fields in
    // HashMap order, so the corpus never misses two. Span = type-name token.
    assert_matches_oracle(
        "struct Point { x: Int, y: Int } fn f() -> Int { let p: Point = Point { x: 1 }; return 0; }",
    );
}

#[test]
fn typecheck_array_element_mismatch() {
    // The first element fixes Int; the `true` reports at its own span.
    assert_matches_oracle("fn f() -> IntArray { return [1, 2, true]; }");
}

#[test]
fn typecheck_array_error_element_suppresses() {
    // `miss` infers "<error>" as the element type, so the later Int element
    // suppresses and the array itself is "<error>" (let suppressed too).
    assert_matches_oracle("fn f() -> Int { let a: IntArray = [miss, 1]; return 0; }");
}

#[test]
fn typecheck_empty_array() {
    // Fixed-message TYPE_ARRAY_EMPTY over the `[]`; the "<error>" result
    // suppresses the let.
    assert_matches_oracle("fn f() -> Int { let a: IntArray = []; return 0; }");
}

#[test]
fn typecheck_index_type_and_base() {
    // A Bool index reports TYPE_INDEX at the index expression.
    assert_matches_oracle("fn f(xs: IntArray) -> Int { return xs[true]; }");
    // A non-array base reports fixed-message TYPE_INDEX_BASE at the base; the
    // "<error>" result suppresses the return check.
    assert_matches_oracle("fn f(n: Int) -> Int { return n[0]; }");
}

#[test]
fn typecheck_index_element_type() {
    // xs[0] yields the element Int: TYPE_RETURN_MISMATCH expected=Bool found=Int.
    assert_matches_oracle("fn f(xs: IntArray) -> Bool { return xs[0]; }");
}

#[test]
fn typecheck_array_len_ok_and_bad_field() {
    // `len` is the one supported array field, typed Int.
    assert_matches_oracle("fn f(xs: IntArray) -> Int { return xs.len; }");
    // Any other field is TYPE_ARRAY_FIELD at the field ident; "<error>"
    // suppresses the return check.
    assert_matches_oracle("fn f(xs: IntArray) -> Int { return xs.size; }");
}

#[test]
fn typecheck_field_base_int_and_recovery_type() {
    // QUIRK: TYPE_FIELD_BASE recovers with Named(field) — the field NAME
    // becomes the type — so the return ALSO reports, with found=x.
    assert_matches_oracle("fn f(n: Int) -> Int { return n.x; }");
}

#[test]
fn typecheck_field_on_unknown_name_not_suppressed() {
    // QUIRK: an "<error>" base is NOT suppressed by the field access — Rust's
    // `let Named(..) = .. else` catches Error too. TYPE_FIELD_BASE plus the
    // found=x return mismatch, same as the Int-base case.
    assert_matches_oracle("fn f() -> Int { return miss.x; }");
}

#[test]
fn typecheck_struct_chain_and_raw_field_type() {
    // QUIRK: struct field types go through parse_type (identity), NOT
    // parse_declared_type — `m: Mystery` keeps Named(Mystery) even though
    // Mystery is unknown. So after the TYPE_UNKNOWN_TYPE from declared-type
    // validation, `return b.m` reports found=Mystery (not suppressed).
    assert_matches_oracle("struct B { m: Mystery } fn f(b: B) -> Int { return b.m; }");
}

#[test]
fn typecheck_assign_immutable_simple_name() {
    // A plain `let` binding is immutable: TYPE_ASSIGN_IMMUTABLE at the target.
    assert_matches_oracle("fn f() -> Int { let x: Int = 1; x = 2; return x; }");
}

#[test]
fn typecheck_assign_mut_simple_name_mismatch() {
    // Mutable binding, wrong value type: expected=Int found=Bool at the value.
    assert_matches_oracle("fn f() -> Int { let mut x: Int = 1; x = true; return x; }");
}

#[test]
fn typecheck_assign_unknown_simple_name() {
    // An unbound simple target reports TYPE_UNKNOWN_NAME (unlike a complex
    // target's unbound root, which stays silent).
    assert_matches_oracle("fn f() -> Int { ghost = 1; return 0; }");
}

#[test]
fn typecheck_assign_field_of_immutable_param() {
    // Parameters are immutable: assigning through `p.x` reports
    // TYPE_ASSIGN_IMMUTABLE at the chain ROOT `p`.
    assert_matches_oracle("struct P { x: Int } fn f(p: P) -> Int { p.x = 1; return 0; }");
}

#[test]
fn typecheck_assign_field_mismatch() {
    // Mutable struct local, wrong value type for the field: expected=Int.
    assert_matches_oracle(
        "struct Point { x: Int, y: Int } fn f() -> Int { let mut p: Point = Point { x: 1, y: 2 }; p.x = true; return 0; }",
    );
}

#[test]
fn typecheck_assign_unknown_field_recovery_chain() {
    // QUIRK chain: inferring the target `p.z` reports TYPE_UNKNOWN_FIELD and
    // recovers with Named(z), so the assign ALSO reports expected=z found=Int.
    assert_matches_oracle(
        "struct Point { x: Int, y: Int } fn f() -> Int { let mut p: Point = Point { x: 1, y: 2 }; p.z = 1; return 0; }",
    );
}

#[test]
fn typecheck_assign_index_of_immutable_param() {
    // Index targets walk to the root too: TYPE_ASSIGN_IMMUTABLE at `xs`.
    assert_matches_oracle("fn f(xs: IntArray) -> Int { xs[0] = 1; return 0; }");
}

#[test]
fn typecheck_for_in_struct_array() {
    // Generalized element extraction: [P{..}, P{..}] types as PArray, so the
    // loop binding is P and `p.x` is Int. Zero lines. (The literal goes
    // through an untyped let because the oracle parser bans struct literals
    // anywhere inside a for-in iterable, even nested in an array literal.)
    assert_matches_oracle(
        "struct P { x: Int } fn f() -> Int { let arr = [P { x: 1 }, P { x: 2 }]; for p in arr { let v: Int = p.x; } return 0; }",
    );
}

// ---------------------------------------------------------------------------
// M4 slice #4 (the last): enum-variant calls and values, match-pattern type
// checking, and match exhaustiveness.
// ---------------------------------------------------------------------------

#[test]
fn typecheck_variant_call_ok() {
    // A correct payload call types as Named(E): zero lines.
    assert_matches_oracle("enum E { A(Int), B } fn f() -> Int { let x: E = E.A(1); return 0; }");
}

#[test]
fn typecheck_variant_call_payload_mismatch() {
    // TYPE_ENUM_VARIANT_PAYLOAD expected=Int found=Bool, span = the argument.
    assert_matches_oracle("enum E { A(Int), B } fn f() -> Int { let x: E = E.A(true); return 0; }");
}

#[test]
fn typecheck_variant_call_arity_then_payload() {
    // Arity reports at the callee span (expected=1 found=2) and then — unlike
    // TYPE_CALL_ARITY's early exit — EVERY argument is still expected against
    // the payload: the second argument adds a PAYLOAD found=Bool line.
    assert_matches_oracle(
        "enum E { A(Int), B } fn f() -> Int { let x: E = E.A(1, true); return 0; }",
    );
}

#[test]
fn typecheck_variant_call_no_payload_with_arg() {
    // A bare variant called with arguments: ARITY expected=0 found=1. The
    // argument is inferred but its type DISCARDED (no payload to expect
    // against), so the bad `1 + true` inside still reports its own operand.
    assert_matches_oracle(
        "enum E { A(Int), B } fn f() -> Int { let x: E = E.B(1 + true); return 0; }",
    );
}

#[test]
fn typecheck_variant_call_unknown_variant_args_unvisited() {
    // Unknown variant: ONE line at the callee span; the argument is never
    // visited, so the bad `1 + true` cannot report. The call still types as
    // Named(E), so the let passes.
    assert_matches_oracle(
        "enum E { A(Int), B } fn f() -> Int { let x: E = E.C(1 + true); return 0; }",
    );
}

#[test]
fn typecheck_enum_value_ok() {
    // A bare variant used as a value is silent and types as Named(E).
    assert_matches_oracle("enum E { A(Int), B } fn f() -> Int { let x: E = E.B; return 0; }");
}

#[test]
fn typecheck_enum_value_needs_call() {
    // A payload variant used UNCALLED: TYPE_ENUM_VARIANT_ARITY kind=uncalled
    // at the field ident; the value still types as Named(E), so the let is ok.
    assert_matches_oracle("enum E { A(Int), B } fn f() -> Int { let x: E = E.A; return 0; }");
}

#[test]
fn typecheck_enum_value_unknown_variant() {
    // TYPE_UNKNOWN_VARIANT at the field ident; recovery is still Named(E), so
    // the let passes — one line total.
    assert_matches_oracle("enum E { A(Int), B } fn f() -> Int { let x: E = E.D; return 0; }");
}

#[test]
fn typecheck_std_enum_variant_call() {
    // `import std.core` grants OptionInt{Some(Int),None}: the Bool payload
    // reports TYPE_ENUM_VARIANT_PAYLOAD expected=Int found=Bool.
    assert_matches_oracle(
        "import std.core; fn f() -> Int { let o: OptionInt = OptionInt.Some(true); return 0; }",
    );
}

#[test]
fn typecheck_enum_shadowed_by_local() {
    // A local named E shadows the enum: `E.x` takes the NORMAL field path on
    // the Int base — TYPE_FIELD_BASE, then the Named(x) recovery makes the
    // return report found=x.
    assert_matches_oracle("enum E { A } fn f(E: Int) -> Int { return E.x; }");
}

#[test]
fn typecheck_match_variant_ok_payload_typed() {
    // The payload binding carries the declared Int: zero lines (and the two
    // variant patterns make the match exhaustive).
    assert_matches_oracle(
        "enum E { A(Int), B } fn f(e: E) -> Int { match e { E.A(n) -> { let y: Int = n; }, E.B -> { } } return 0; }",
    );
}

#[test]
fn typecheck_match_pattern_type_mismatch() {
    // A bool pattern on an Int value: TYPE_MATCH_PATTERN expected=Bool
    // found=Int at the match VALUE expression.
    assert_matches_oracle("fn f(n: Int) -> Int { match n { true -> { }, _ -> { } } return 0; }");
}

#[test]
fn typecheck_match_int_pattern_ok() {
    assert_matches_oracle("fn f(n: Int) -> Int { match n { 1 -> { }, _ -> { } } return 0; }");
}

#[test]
fn typecheck_match_unknown_enum_pattern() {
    // A variant pattern naming an unknown enum on an Int value: the pattern
    // expect runs FIRST (TYPE_MATCH_PATTERN expected=Ghost found=Int), then
    // TYPE_UNKNOWN_ENUM — two lines, both at the value span.
    assert_matches_oracle("fn f(n: Int) -> Int { match n { Ghost.A -> { }, _ -> { } } return 0; }");
}

#[test]
fn typecheck_match_unknown_variant_pattern() {
    // TYPE_UNKNOWN_VARIANT name=Z at the value span; the wildcard keeps the
    // match exhaustive.
    assert_matches_oracle(
        "enum E { A } fn f(e: E) -> Int { match e { E.Z -> { }, _ -> { } } return 0; }",
    );
}

#[test]
fn typecheck_match_pattern_arity_unbound() {
    // E.A carries a payload but binds nothing: TYPE_ENUM_PATTERN_ARITY
    // kind=unbound. The mis-bound pattern STILL covers A, so exhaustiveness
    // passes — one line total.
    assert_matches_oracle(
        "enum E { A(Int), B } fn f(e: E) -> Int { match e { E.A -> { }, E.B -> { } } return 0; }",
    );
}

#[test]
fn typecheck_match_pattern_arity_spurious_no_binding() {
    // E.B carries no payload but binds x: kind=spurious AND x is NOT bound —
    // its arm-body use infers "<error>" and the let suppresses. One line.
    assert_matches_oracle(
        "enum E { A(Int), B } fn f(e: E) -> Int { match e { E.A(n) -> { }, E.B(x) -> { let z: Int = x; } } return 0; }",
    );
}

#[test]
fn typecheck_match_non_exhaustive_enum() {
    // B is uncovered: TYPE_MATCH_NON_EXHAUSTIVE name=E at the value span.
    assert_matches_oracle(
        "enum E { A(Int), B } fn f(e: E) -> Int { match e { E.A(n) -> { } } return 0; }",
    );
}

#[test]
fn typecheck_match_wildcard_exhaustive() {
    // `_` covers everything: zero lines.
    assert_matches_oracle(
        "enum E { A(Int), B } fn f(e: E) -> Int { match e { E.A(n) -> { }, _ -> { } } return 0; }",
    );
}

#[test]
fn typecheck_match_name_pattern_exhaustive() {
    // A name pattern covers everything too (and binds the value's type).
    assert_matches_oracle(
        "enum E { A(Int), B } fn f(e: E) -> Int { match e { other -> { let x: E = other; } } return 0; }",
    );
}

#[test]
fn typecheck_match_bool_non_exhaustive() {
    // Bool needs BOTH literals (or `_`): the one-arm match reports the
    // fixed-message TYPE_MATCH_NON_EXHAUSTIVE...
    assert_matches_oracle("fn f(b: Bool) -> Int { match b { true -> { } } return 0; }");
    // ...and the two-arm match is clean.
    assert_matches_oracle(
        "fn f(b: Bool) -> Int { match b { true -> { }, false -> { } } return 0; }",
    );
}

#[test]
fn typecheck_std_enum_match_exhaustive() {
    // Std variants drive exhaustiveness like user ones, and Some's payload
    // binds Int: zero lines.
    assert_matches_oracle(
        "import std.core; fn f(o: OptionInt) -> Int { match o { OptionInt.Some(v) -> { let z: Int = v; }, OptionInt.None -> { } } return 0; }",
    );
}

#[test]
fn typecheck_match_wrong_enum_pattern_covered() {
    // The F.A pattern mismatches the E value (TYPE_MATCH_PATTERN expected=F
    // found=E) and contributes NOTHING to coverage — but E.A covers E's only
    // variant, so no NON_EXHAUSTIVE line. One line total.
    assert_matches_oracle(
        "enum E { A } enum F { A } fn f(e: E) -> Int { match e { F.A -> { }, E.A -> { } } return 0; }",
    );
}
