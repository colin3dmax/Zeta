// M6 self-hosting milestone (slice 1): the Zeta-written MIR-arena evaluator in
// testdata/selfhost/arena_frontend.zeta must run `main()` and produce the same
// result string as the Rust oracle `zeta::run_source(source).to_string()`.
//
// This slice covers scalar values (Int / Bool / Unit), arithmetic / bitwise /
// comparison / boolean (with short-circuit), control flow (if / while /
// for-range / for-c), and user-function calls with an isolated call frame.
// String / Struct / Enum / Array values, the field/index Store places,
// for-in / match statements, and std builtins are reserved for slice 2/3.

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

/// Evaluate `program_source` through the Zeta-written arena evaluator
/// (`run_main_via_arena`) running inside the Zeta interpreter, and return the
/// result string it produces.
fn zeta_run(source: &str) -> String {
    let caller = format!(
        r#"
module selfhost.caller;
import selfhost.arena_frontend;

fn main() -> String {{
  let source: String = {literal};
  return selfhost.arena_frontend.run_main_via_arena(source);
}}
"#,
        literal = zeta_string_literal(source),
    );

    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/selfhost/arena_frontend.zeta",
            include_str!("../testdata/selfhost/arena_frontend.zeta"),
        ),
        source_file("testdata/selfhost/caller.zeta", &caller),
    ])
    .expect("arena evaluator caller should run");

    value.to_string()
}

/// The Rust oracle: run `main()` via the MIR runtime and stringify the result.
fn oracle_run(source: &str) -> String {
    zeta::run_source(source)
        .expect("oracle should run")
        .to_string()
}

fn assert_runs(source: &str) {
    let zeta = zeta_run(source);
    let oracle = oracle_run(source);
    assert_eq!(
        zeta, oracle,
        "\n--- source ---\n{source}\n--- zeta ---\n{zeta}\n--- oracle ---\n{oracle}\n"
    );
}

// --- Targeted slice-1 probes (existing run_*.zeta corpus). Each is asserted
// individually so a per-category regression is easy to localize. ---

#[test]
fn run_matches_oracle_on_basic() {
    assert_runs(include_str!("../testdata/run_basic.zeta"));
}

#[test]
fn run_matches_oracle_on_branch() {
    assert_runs(include_str!("../testdata/run_branch.zeta"));
}

#[test]
fn run_matches_oracle_on_neg() {
    assert_runs(include_str!("../testdata/run_neg.zeta"));
}

#[test]
fn run_matches_oracle_on_elif() {
    assert_runs(include_str!("../testdata/run_elif.zeta"));
}

#[test]
fn run_matches_oracle_on_mod() {
    assert_runs(include_str!("../testdata/run_mod.zeta"));
}

#[test]
fn run_matches_oracle_on_bitwise() {
    assert_runs(include_str!("../testdata/run_bitwise.zeta"));
}

#[test]
fn run_matches_oracle_on_compound() {
    assert_runs(include_str!("../testdata/run_compound.zeta"));
}

#[test]
fn run_matches_oracle_on_compare() {
    assert_runs(include_str!("../testdata/run_compare.zeta"));
}

#[test]
fn run_matches_oracle_on_bool_logic() {
    assert_runs(include_str!("../testdata/run_bool_logic.zeta"));
}

#[test]
fn run_matches_oracle_on_call() {
    assert_runs(include_str!("../testdata/run_call.zeta"));
}

#[test]
fn run_matches_oracle_on_mut() {
    assert_runs(include_str!("../testdata/run_mut.zeta"));
}

#[test]
fn run_matches_oracle_on_loop_control() {
    assert_runs(include_str!("../testdata/run_loop_control.zeta"));
}

#[test]
fn run_matches_oracle_on_forrange() {
    assert_runs(include_str!("../testdata/run_forrange.zeta"));
}

#[test]
fn run_matches_oracle_on_forc() {
    assert_runs(include_str!("../testdata/run_forc.zeta"));
}

#[test]
fn run_matches_oracle_on_array() {
    assert_runs(include_str!("../testdata/run_array.zeta"));
}

#[test]
fn run_matches_oracle_on_assign() {
    assert_runs(include_str!("../testdata/run_assign.zeta"));
}

#[test]
fn run_matches_oracle_on_for() {
    assert_runs(include_str!("../testdata/run_for.zeta"));
}

#[test]
fn run_matches_oracle_on_struct() {
    assert_runs(include_str!("../testdata/run_struct.zeta"));
}

// --- Slice-3 corpus: enum/match, string literals, std builtins, arrays. ---

#[test]
fn run_matches_oracle_on_enum() {
    assert_runs(include_str!("../testdata/run_enum.zeta"));
}

#[test]
fn run_matches_oracle_on_enum_payload() {
    assert_runs(include_str!("../testdata/run_enum_payload.zeta"));
}

#[test]
fn run_matches_oracle_on_match() {
    assert_runs(include_str!("../testdata/run_match.zeta"));
}

#[test]
fn run_matches_oracle_on_std_core() {
    assert_runs(include_str!("../testdata/run_std_core.zeta"));
}

#[test]
fn run_matches_oracle_on_string_scan() {
    assert_runs(include_str!("../testdata/run_string_scan.zeta"));
}

#[test]
fn run_matches_oracle_on_string_build() {
    assert_runs(include_str!("../testdata/run_string_build.zeta"));
}

#[test]
fn run_matches_oracle_on_io_path_diagnostic() {
    assert_runs(include_str!("../testdata/run_io_path_diagnostic.zeta"));
}

#[test]
fn run_matches_oracle_on_array_builder() {
    assert_runs(include_str!("../testdata/run_array_builder.zeta"));
}

// --- Self-built slice-1 probes (testdata/selfhost_run/): recursion + call
// frame isolation, nested loop control, multi-param chains, short-circuit,
// negation/bitwise mix, deep nested loops. ---

fn zeta_paths(dir: &str) -> Vec<std::path::PathBuf> {
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .expect("probe dir should exist")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().map(|x| x == "zeta").unwrap_or(false))
        .collect();
    paths.sort();
    paths
}

#[test]
fn run_matches_oracle_on_selfhost_run_probes() {
    let paths = zeta_paths("testdata/selfhost_run");
    assert_eq!(paths.len(), 19, "expected 19 selfhost_run probes");
    let mut failures = Vec::new();
    for path in &paths {
        let source = std::fs::read_to_string(path).expect("probe source should read");
        let zeta = zeta_run(&source);
        let oracle = oracle_run(&source);
        if zeta != oracle {
            failures.push(format!(
                "{}: zeta={zeta:?} oracle={oracle:?}",
                path.display()
            ));
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
