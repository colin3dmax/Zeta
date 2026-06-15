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
    assert_matches_oracle("fn f() -> Bool { let r: Bool = a || b && c == d + e * f; return r; }");
}

#[test]
fn arena_matches_oracle_on_bool_literals() {
    assert_matches_oracle("fn f() -> Bool { let t: Bool = true; let g: Bool = false; return t; }");
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
    assert_matches_oracle("fn f() -> Int { while c { x = x + 1; if d { x = x - 1; } } return x; }");
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

// --- Batch 3: top-level items (import/struct/enum) + postfix/compound
// expressions (call/index/field-access/array & struct literals/strings) ---

#[test]
fn arena_matches_oracle_on_imports() {
    assert_matches_oracle("import a.b.c;");
    assert_matches_oracle("import a.b as x;");
    assert_matches_oracle("export import a.b;");
    assert_matches_oracle("export import a.b as y;");
    assert_matches_oracle(
        "import std.core; import demo.util as u; export import shared.api; export import shared.io as io;",
    );
}

#[test]
fn arena_matches_oracle_on_structs() {
    assert_matches_oracle("struct Point { x: Int, y: Int, }");
    assert_matches_oracle("export struct Pair { a: String, b: Bool, }");
    assert_matches_oracle("struct Empty { }");
}

#[test]
fn arena_matches_oracle_on_enums() {
    assert_matches_oracle("enum Color { Red, Green, Blue, }");
    assert_matches_oracle("enum Shape { Dot, Box(Int), Tagged(String), }");
    assert_matches_oracle("export enum Mixed { A, B(Int), C, }");
}

#[test]
fn arena_matches_oracle_on_mixed_items() {
    assert_matches_oracle(
        "module demo.app; import std.core; struct P { x: Int, } enum E { A, B(Int), } fn main() -> Int { return 1; }",
    );
}

#[test]
fn arena_matches_oracle_on_string_literals() {
    assert_matches_oracle("fn f() -> String { let s: String = \"hello\"; return s; }");
    assert_matches_oracle(
        "fn f() -> String { let s: String = \"a\\nb\\tc\\\\d\\\"e\"; return s; }",
    );
}

#[test]
fn arena_matches_oracle_on_calls() {
    assert_matches_oracle("fn f() -> Int { let a: Int = g(x, y); return a; }");
    assert_matches_oracle("fn f() -> Int { let a: Int = k(); return a; }");
    assert_matches_oracle("fn f() -> Int { let a: Int = a.b.h(p); return a; }");
    assert_matches_oracle("fn f() -> Int { let a: Int = f(g(h())); return a; }");
}

#[test]
fn arena_matches_oracle_on_index() {
    assert_matches_oracle("fn f() -> Int { let a: Int = arr[i]; return a; }");
    assert_matches_oracle("fn f() -> Int { let a: Int = m[i][j]; return a; }");
}

#[test]
fn arena_matches_oracle_on_field_access() {
    assert_matches_oracle("fn f() -> Int { let a: Int = x.b.c; return a; }");
}

#[test]
fn arena_matches_oracle_on_array_literals() {
    assert_matches_oracle("fn f() -> Int { let a: Int = []; return a; }");
    assert_matches_oracle("fn f() -> Int { let a: Int = [1, 2, 3]; return a; }");
    assert_matches_oracle("fn f() -> Int { let a: Int = [g(x), y[0]]; return a; }");
}

#[test]
fn arena_matches_oracle_on_struct_literals() {
    assert_matches_oracle("fn f() -> Int { let a: Int = Point { x: 1, y: 2 }; return a; }");
    assert_matches_oracle("fn f() -> Int { let a: Int = X {}; return a; }");
    assert_matches_oracle(
        "fn f() -> Int { let a: Int = Outer { inner: Inner { v: 3 } }; return a; }",
    );
}

#[test]
fn arena_matches_oracle_on_interleaved_postfix() {
    assert_matches_oracle("fn f() -> Int { let a: Int = a.b(c)[d].e; return a; }");
    assert_matches_oracle("fn f() -> Int { let a: Int = f(x)[0]; return a; }");
    assert_matches_oracle("fn f() -> Int { let a: Int = make().field; return a; }");
    assert_matches_oracle("fn f() -> Int { let a: Int = make()[i][j]; return a; }");
    assert_matches_oracle("fn f() -> Int { let a: Int = arr[i].field; return a; }");
}

#[test]
fn arena_matches_oracle_on_struct_literal_not_in_condition() {
    // `if`/`while` conditions must NOT treat `Name {` as a struct literal.
    assert_matches_oracle("fn f() -> Int { if x { return 1; } return 0; }");
    assert_matches_oracle(
        "fn f() -> Int { let a: Int = P { v: 1 }; if a { return 1; } return 0; }",
    );
}

#[test]
fn arena_matches_oracle_on_empty_return_and_no_return_type() {
    assert_matches_oracle("fn f() { return; }");
    assert_matches_oracle("module it.basic; fn main() { return; }");
}

// --- Regression: targeted stage1 parity probe groups. (The full corpus is
// gated by arena_matches_oracle_on_all_stage1_parity_probes below; these keep
// per-category failures easy to localize.)
fn assert_probe(path: &str) {
    let source = std::fs::read_to_string(path).expect("probe source should read");
    assert_matches_oracle(&source);
}

#[test]
fn arena_matches_oracle_on_operator_probes() {
    // op_13 (call/index/field) and op_chain_boundaries (call/string) are now
    // covered by this batch's postfix/string support.
    for name in [
        "op_01",
        "op_02",
        "op_03",
        "op_04",
        "op_05",
        "op_06",
        "op_07",
        "op_08",
        "op_09",
        "op_10",
        "op_11",
        "op_12",
        "op_13",
        "op_14",
        "op_15",
        "op_chain_boundaries",
    ] {
        assert_probe(&format!("testdata/stage1_parity/{name}.zeta"));
    }
}

#[test]
fn arena_matches_oracle_on_item_probes() {
    for n in 1..=12 {
        assert_probe(&format!("testdata/stage1_parity/it_{n:02}.zeta"));
    }
}

#[test]
fn arena_matches_oracle_on_postfix_probes() {
    for n in 1..=17 {
        assert_probe(&format!("testdata/stage1_parity/px_{n:02}.zeta"));
    }
}

#[test]
fn arena_matches_oracle_on_literal_probes() {
    for n in 1..=16 {
        assert_probe(&format!("testdata/stage1_parity/ll_{n:02}.zeta"));
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
    for n in 1..=15 {
        assert_probe(&format!("testdata/stage1_parity/cf_{n:02}.zeta"));
    }
    for name in [
        "elif_01",
        "for_01",
        "forc_01",
        "forrange_01",
        "control_flow_core",
    ] {
        assert_probe(&format!("testdata/stage1_parity/{name}.zeta"));
    }
}

#[test]
fn arena_matches_oracle_on_match_probes() {
    for n in 1..=14 {
        assert_probe(&format!("testdata/stage1_parity/mt_{n:02}.zeta"));
    }
}

#[test]
fn arena_matches_oracle_on_assignment_probes() {
    for name in ["asn_01", "cae_01"] {
        assert_probe(&format!("testdata/stage1_parity/{name}.zeta"));
    }
}

// --- Batch 4 (final): match / for (in / range / C-style) / break / continue /
// complex assignment targets / compound assignment ---

#[test]
fn arena_matches_oracle_on_break_continue() {
    assert_matches_oracle("fn f() -> Int { while c { break; } return 0; }");
    assert_matches_oracle("fn f() -> Int { while c { continue; } return 0; }");
    assert_matches_oracle(
        "fn f() -> Int { while c { if a { break; } else { continue; } } return 0; }",
    );
}

#[test]
fn arena_matches_oracle_on_for_in() {
    assert_matches_oracle("fn f() -> Int { for x in arr { y = x; } return 0; }");
    assert_matches_oracle("fn f() -> Int { for item in xs.items { use(item); } return 0; }");
}

#[test]
fn arena_matches_oracle_on_for_range() {
    assert_matches_oracle("fn f() -> Int { for i in a..b { y = i; } return 0; }");
    assert_matches_oracle("fn f() -> Int { for i in 0..n { s = s + i; } return 0; }");
}

#[test]
fn arena_matches_oracle_on_for_c() {
    assert_matches_oracle(
        "fn f() -> Int { for (let mut i: Int = 0; i < n; i = i + 1) { s = s + i; } return s; }",
    );
    assert_matches_oracle(
        "fn f() -> Int { for (let mut i: Int = 0; i < n; i += 1) { s = s + i; } return s; }",
    );
}

#[test]
fn arena_matches_oracle_on_match_patterns() {
    assert_matches_oracle(
        "fn f(x: Int) -> Int { match x { 1 -> { return 1; }, 2 -> { return 2; } } }",
    );
    assert_matches_oracle(
        "fn f(x: String) -> Int { match x { \"a\" -> { return 1; }, _ -> { return 0; } } }",
    );
    assert_matches_oracle(
        "fn f(x: Bool) -> Int { match x { true -> { return 1; }, false -> { return 0; } } }",
    );
    assert_matches_oracle("fn f(x: Int) -> Int { match x { n -> { return n; } } }");
    assert_matches_oracle(
        "fn f(c: Color) -> Int { match c { Color.Red -> { return 1; }, Color.Green -> { return 2; }, _ -> { return 0; } } }",
    );
    assert_matches_oracle(
        "fn f(s: Shape) -> Int { match s { Shape.Box(b) -> { return b; }, Shape.Tag(t) -> { return t; } } }",
    );
}

#[test]
fn arena_matches_oracle_on_nested_match() {
    assert_matches_oracle(
        "fn f(x: Int, y: Int) -> Int { match x { 1 -> { match y { 2 -> { return 3; }, _ -> { return 4; } } }, _ -> { return 0; } } }",
    );
}

#[test]
fn arena_matches_oracle_on_complex_assign_targets() {
    assert_matches_oracle("fn f() -> Int { a.b = x; return 0; }");
    assert_matches_oracle("fn f() -> Int { a[i] = x; return 0; }");
    assert_matches_oracle("fn f() -> Int { a.b.c = x; return 0; }");
    assert_matches_oracle("fn f() -> Int { a[i][j] = x; return 0; }");
    assert_matches_oracle("fn f() -> Int { a.b[i].c = x; return 0; }");
}

#[test]
fn arena_matches_oracle_on_compound_assign_simple() {
    assert_matches_oracle("fn f() -> Int { a += 1; return 0; }");
    assert_matches_oracle("fn f() -> Int { a -= 2; return 0; }");
    assert_matches_oracle("fn f() -> Int { a *= 3; return 0; }");
    assert_matches_oracle("fn f() -> Int { a /= 4; return 0; }");
    assert_matches_oracle("fn f() -> Int { a %= 5; return 0; }");
}

#[test]
fn arena_matches_oracle_on_compound_assign_complex() {
    assert_matches_oracle("fn f() -> Int { a.b += 1; return 0; }");
    assert_matches_oracle("fn f() -> Int { a[i] *= 2; return 0; }");
    assert_matches_oracle("fn f() -> Int { a.b.c -= n + 1; return 0; }");
}

// Full-coverage gate: every Stage1 parity probe must produce arena dump text
// identical to the oracle. This is the M2 completeness milestone — the arena
// structural frontend now matches the text-based Stage1 frontend across the
// entire probe corpus (match / for / break / continue / compound + complex
// assignment included).
#[test]
fn arena_matches_oracle_on_all_stage1_parity_probes() {
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir("testdata/stage1_parity")
        .expect("stage1_parity dir should exist")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().map(|x| x == "zeta").unwrap_or(false))
        .collect();
    paths.sort();
    assert_eq!(paths.len(), 243, "expected 243 stage1 parity probes");

    let mut failures = Vec::new();
    for path in &paths {
        let source = std::fs::read_to_string(path).expect("probe source should read");
        let oracle = zeta::dump_ast(&source).expect("oracle ast-dump should succeed");
        let arena = arena_dump(&source);
        if arena.trim_end() != oracle.trim_end() {
            failures.push(path.display().to_string());
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} probes diverged from the oracle:\n{}",
        failures.len(),
        paths.len(),
        failures.join("\n")
    );
}

#[test]
fn arena_matches_oracle_on_review_kitchen_sink() {
    assert_matches_oracle(
        "fn f(a: Int, b: Int) -> Bool { let mut r: Int = a || b && a == b + a * b; if a < b { r = a; } else if a > b { r = b; } else { r = 0; } while r < a & b | a { r = r + 1; } return !r == false && -a < ~b; }",
    );
}

#[test]
fn arena_matches_oracle_on_items_and_postfix_kitchen_sink() {
    assert_matches_oracle(
        "module demo.app; import demo.math; import demo.text as txt; export struct User { id: Int, name: String } enum Result { Ok(Int), Err(String), None } export fn build(n: Int) -> User { let items: IntArray = [1, n + 1, compute(n)[0]]; let u: User = User { id: next(), name: \"a\\nb\" }; return User { id: u.child.items[0], name: txt.fmt(u.name, [n][0]) }; }",
    );
}

#[test]
fn arena_matches_oracle_on_final_kitchen_sink() {
    assert_matches_oracle(
        "fn f(n: Int) -> Int { let mut s: Int = 0; for i in 0..n { s += i; } for x in [1,2,3] { s = s + x; } for (let mut j: Int = 0; j < n; j += 1) { if j == 2 { continue; } s.acc[j] = j; } match s { 0 -> { return 0; }, _ -> { match n { 1 -> { break; }, _ -> { return s; } } } } return s; }",
    );
}

// --- Float (P1 feature back-ported into the self-hosting frontend) ----------

#[test]
fn arena_matches_oracle_on_float_literal() {
    assert_matches_oracle("fn f() -> Float { let x: Float = 1.5; return x; }");
}

#[test]
fn arena_matches_oracle_on_float_arithmetic() {
    assert_matches_oracle(
        "fn f() -> Float { let x: Float = 3.0; let y: Float = 2.0; return x * y - x / y; }",
    );
}

#[test]
fn arena_matches_oracle_on_float_and_int_distinct() {
    // A float `1..2`-style disambiguation guard: `1.5` is a float, `0..3` is a range.
    assert_matches_oracle(
        "fn f() -> Int { let a: Float = 0.5; let mut s: Int = 0; for i in 0..3 { s = s + i; } return s; }",
    );
}

// --- Tuple (P2 feature back-ported into the self-hosting frontend) ----------

#[test]
fn arena_matches_oracle_on_tuple_literal() {
    assert_matches_oracle("fn f() -> Int { let t = (10, 20, 30); return t.0 + t.1 + t.2; }");
}

#[test]
fn arena_matches_oracle_on_tuple_grouping_distinct() {
    // `(a)` is grouping (not a 1-tuple); `(a, b)` is a tuple.
    assert_matches_oracle("fn f() -> Int { let x = (3 + 4) * 2; let t = (1, 2); return x + t.0; }");
}

#[test]
fn arena_matches_oracle_on_tuple_nested_index() {
    // `t.1.0` lexes as a single Float token and must split into two indices.
    assert_matches_oracle("fn f() -> Int { let t = (1, (2, 3)); return t.0 + t.1.0 + t.1.1; }");
}

#[test]
fn arena_matches_oracle_on_tuple_heterogeneous() {
    assert_matches_oracle("fn f() -> Int { let t = (7, true); if t.1 { return t.0; } return 0; }");
}

// --- Generics (P4 feature back-ported into the self-hosting frontend) --------

#[test]
fn arena_matches_oracle_on_generic_identity() {
    assert_matches_oracle("fn id<T>(x: T) -> T { return x; } fn main() -> Int { return id(5); }");
}

#[test]
fn arena_matches_oracle_on_generic_two_params() {
    assert_matches_oracle(
        "fn pick<A, B>(a: A, b: B) -> A { return a; } fn main() -> Int { return pick(9, 3); }",
    );
}
