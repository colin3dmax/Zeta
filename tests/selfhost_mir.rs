// M5 self-hosting milestone: AST arena -> MIR arena lowering + mir-dump
// written in Zeta (testdata/selfhost/arena_frontend.zeta) must produce dump
// text byte-for-byte identical to the Rust oracle.
//
// Gate A (lowering parity, parse-only oracle): for every Stage1 parity probe
// plus the dedicated MIR probes in testdata/selfhost_mir/,
// `mir_dump_via_arena(<source>)` must equal `zeta::mir::dump(&module)`
// (lowering with NO external enums).
//
// Gate B (pipeline parity): for full-pipeline-valid programs,
// `mir_dump_pipeline_via_arena(<source>)` must equal `zeta::dump_mir(<source>)`
// (std imports inject the OptionInt/ResultInt/ResultString external enums).

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

/// Run the arena frontend's MIR pipeline over `program_source` inside the Zeta
/// interpreter through the given export and return the dump string it produces.
fn arena_mir_dump(program_source: &str, entry: &str) -> String {
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
        entry = entry,
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

fn assert_lowering_matches_oracle(program_source: &str) {
    let module = zeta::parse_source(program_source).expect("oracle parse should succeed");
    let oracle = zeta::mir::dump(&module);
    let arena = arena_mir_dump(program_source, "mir_dump_via_arena");
    assert_eq!(
        arena.trim_end(),
        oracle.trim_end(),
        "\n--- source ---\n{program_source}\n--- arena ---\n{arena}\n--- oracle ---\n{oracle}\n"
    );
}

fn assert_pipeline_matches_oracle(program_source: &str) {
    let oracle =
        zeta::dump_mir(program_source).expect("full-pipeline mir-dump oracle should succeed");
    let arena = arena_mir_dump(program_source, "mir_dump_pipeline_via_arena");
    assert_eq!(
        arena.trim_end(),
        oracle.trim_end(),
        "\n--- source ---\n{program_source}\n--- arena ---\n{arena}\n--- oracle ---\n{oracle}\n"
    );
}

fn assert_probe(path: &str) {
    let source = std::fs::read_to_string(path).expect("probe source should read");
    assert_lowering_matches_oracle(&source);
}

// --- Targeted gate-A probes (the full corpora are gated below; these keep
// per-category failures easy to localize). ---

#[test]
fn mir_matches_oracle_on_basic_function() {
    assert_lowering_matches_oracle("fn main() -> Int { let x: Int = 1 + 2 * 3; return x; }");
}

#[test]
fn mir_matches_oracle_on_lambda() {
    // Closure (P3): lambda lowers to `_tN = lambda |x|` + body; indirect call
    // `f(5)` dumps as a normal `call f(...)`.
    assert_lowering_matches_oracle(
        "fn main() -> Int { let n: Int = 10; let f = |x: Int| x + n; return f(5); }",
    );
}

#[test]
fn mir_matches_oracle_on_generic() {
    // Generics (P4): `<T>` is consumed; the generic body lowers with T-typed
    // params (mir-dump must still match the Rust oracle).
    assert_lowering_matches_oracle(
        "fn id<T>(x: T) -> T { return x; } fn main() -> Int { return id(5); }",
    );
}

#[test]
fn mir_matches_oracle_on_tuple() {
    // Tuple (P2) back-ported: tuple literal lowers to `tuple (...)`, `.N` to field.
    assert_lowering_matches_oracle(
        "fn f() -> Int { let t = (1, (2, 3)); return t.0 + t.1.0 + t.1.1; }",
    );
}

#[test]
fn mir_matches_oracle_on_float() {
    // Float (P1) back-ported into the self-hosting frontend: lowering + mir-dump
    // ("const Float ...") must match the Rust oracle.
    assert_lowering_matches_oracle(
        "fn f() -> Float { let x: Float = 1.5; let y: Float = 2.0; return x * y - x / y; }",
    );
}

#[test]
fn mir_matches_oracle_on_enum_variant_probe() {
    assert_probe("testdata/selfhost_mir/mir_enum_variants.zeta");
}

#[test]
fn mir_matches_oracle_on_enum_quirk_probe() {
    // `E.A(1, 2)` keeps only the first payload argument; `E.x` stays a
    // FieldAccess; `F.A(3)` stays a Call.
    assert_probe("testdata/selfhost_mir/mir_enum_quirk.zeta");
}

#[test]
fn mir_matches_oracle_on_drop_probe() {
    assert_probe("testdata/selfhost_mir/mir_drop.zeta");
}

#[test]
fn mir_matches_oracle_on_forrange_probe() {
    assert_probe("testdata/selfhost_mir/mir_forrange.zeta");
}

#[test]
fn mir_matches_oracle_on_forc_probe() {
    assert_probe("testdata/selfhost_mir/mir_forc.zeta");
}

#[test]
fn mir_matches_oracle_on_match_patterns_probe() {
    assert_probe("testdata/selfhost_mir/mir_match_patterns.zeta");
}

#[test]
fn mir_matches_oracle_on_places_probe() {
    assert_probe("testdata/selfhost_mir/mir_places.zeta");
}

#[test]
fn mir_matches_oracle_on_struct_literals_probe() {
    assert_probe("testdata/selfhost_mir/mir_struct_literals.zeta");
}

#[test]
fn mir_matches_oracle_on_arrays_chains_probe() {
    assert_probe("testdata/selfhost_mir/mir_arrays_chains.zeta");
}

#[test]
fn mir_matches_oracle_on_while_control_probe() {
    assert_probe("testdata/selfhost_mir/mir_while_control.zeta");
}

#[test]
fn mir_matches_oracle_on_bare_return_probe() {
    assert_probe("testdata/selfhost_mir/mir_bare_return.zeta");
}

#[test]
fn mir_matches_oracle_on_unknown_names_probe() {
    assert_probe("testdata/selfhost_mir/mir_unknown_names.zeta");
}

#[test]
fn mir_matches_oracle_on_if_else_probe() {
    assert_probe("testdata/selfhost_mir/mir_if_else.zeta");
}

// --- Gate A: full corpora. Every Stage1 parity probe and every dedicated MIR
// probe must lower + dump byte-for-byte identically to `zeta::mir::dump`. ---

fn zeta_paths(dir: &str) -> Vec<std::path::PathBuf> {
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .expect("probe dir should exist")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().map(|x| x == "zeta").unwrap_or(false))
        .collect();
    paths.sort();
    paths
}

fn assert_corpus_matches_oracle(paths: &[std::path::PathBuf]) {
    let mut failures = Vec::new();
    for path in paths {
        let source = std::fs::read_to_string(path).expect("probe source should read");
        let module = zeta::parse_source(&source).expect("oracle parse should succeed");
        let oracle = zeta::mir::dump(&module);
        let arena = arena_mir_dump(&source, "mir_dump_via_arena");
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
fn mir_matches_oracle_on_all_stage1_parity_probes() {
    let paths = zeta_paths("testdata/stage1_parity");
    assert_eq!(paths.len(), 243, "expected 243 stage1 parity probes");
    assert_corpus_matches_oracle(&paths);
}

#[test]
fn mir_matches_oracle_on_all_selfhost_mir_probes() {
    let paths = zeta_paths("testdata/selfhost_mir");
    assert_eq!(paths.len(), 13, "expected 13 selfhost_mir probes");
    assert_corpus_matches_oracle(&paths);
}

// --- Gate B: full-pipeline parity against `zeta::dump_mir` (resolve +
// typecheck + lowering with std external enums + verify). ---

#[test]
fn mir_pipeline_matches_oracle_on_runtime_sources() {
    for source in [
        include_str!("../testdata/run_mut.zeta"),
        include_str!("../testdata/run_match.zeta"),
        include_str!("../testdata/run_loop_control.zeta"),
    ] {
        assert_pipeline_matches_oracle(source);
    }
}

#[test]
fn mir_pipeline_matches_oracle_on_std_core_enums() {
    // `import std.core` injects OptionInt/ResultInt: the external enums dump
    // after the user enums and `OptionInt.Some(..)` / `OptionInt.None` lower
    // to EnumVariant.
    assert_pipeline_matches_oracle(
        r#"import std.core;

fn pick(flag: Bool) -> OptionInt {
  if flag {
    return OptionInt.Some(1);
  }
  return OptionInt.None;
}

fn main() -> Int {
  let o: OptionInt = pick(true);
  match o {
    OptionInt.Some(v) -> {
      return v;
    },
    OptionInt.None -> {
      return 0;
    },
  }
  return 0;
}
"#,
    );
}

#[test]
fn mir_pipeline_matches_oracle_on_std_core_and_io_enums() {
    // Both std imports together: the dump header must list all three external
    // enums in name order (OptionInt, ResultInt, ResultString).
    assert_pipeline_matches_oracle(
        r#"import std.core;
import std.io;

fn main() -> Int {
  let o: OptionInt = OptionInt.Some(7);
  match o {
    OptionInt.Some(v) -> {
      return v;
    },
    _ -> {
      return 0;
    },
  }
  return 0;
}
"#,
    );
}
