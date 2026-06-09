#[test]
fn run_executes_main_integer_result() {
    let value =
        zeta::run_source(include_str!("../testdata/run_basic.zeta")).expect("program should run");
    assert_eq!(value.to_string(), "42");
}

#[test]
fn run_executes_if_return() {
    let value =
        zeta::run_source(include_str!("../testdata/run_branch.zeta")).expect("program should run");
    assert_eq!(value.to_string(), "7");
}

#[test]
fn run_executes_unary_negation() {
    let value =
        zeta::run_source(include_str!("../testdata/run_neg.zeta")).expect("program should run");
    assert_eq!(value.to_string(), "5");
}

#[test]
fn run_executes_else_if_chain() {
    let value =
        zeta::run_source(include_str!("../testdata/run_elif.zeta")).expect("program should run");
    assert_eq!(value.to_string(), "2");
}

#[test]
fn run_executes_complex_assignment_targets() {
    // p.x = 50; arr[2] = p.x; → 50 + 50 = 100
    let value =
        zeta::run_source(include_str!("../testdata/run_assign.zeta")).expect("program should run");
    assert_eq!(value.to_string(), "100");
}

#[test]
fn run_executes_modulo() {
    // 17 % 5 + 100 % 7 = 2 + 2 = 4
    let value =
        zeta::run_source(include_str!("../testdata/run_mod.zeta")).expect("program should run");
    assert_eq!(value.to_string(), "4");
}

#[test]
fn run_executes_compound_assignment() {
    // a=5; a+=3 → 8; a*=2 → 16; a-=1 → 15
    let value = zeta::run_source(include_str!("../testdata/run_compound.zeta"))
        .expect("program should run");
    assert_eq!(value.to_string(), "15");
}

#[test]
fn run_executes_function_call() {
    let value =
        zeta::run_source(include_str!("../testdata/run_call.zeta")).expect("program should run");
    assert_eq!(value.to_string(), "42");
}

#[test]
fn run_executes_mutable_assignment() {
    let value =
        zeta::run_source(include_str!("../testdata/run_mut.zeta")).expect("program should run");
    assert_eq!(value.to_string(), "42");
}

#[test]
fn run_executes_comparison_conditions() {
    let value =
        zeta::run_source(include_str!("../testdata/run_compare.zeta")).expect("program should run");
    assert_eq!(value.to_string(), "42");
}

#[test]
fn run_executes_boolean_logic_conditions() {
    let value = zeta::run_source(include_str!("../testdata/run_bool_logic.zeta"))
        .expect("program should run");
    assert_eq!(value.to_string(), "42");
}

#[test]
fn run_executes_loop_break_and_continue() {
    let value = zeta::run_source(include_str!("../testdata/run_loop_control.zeta"))
        .expect("program should run");
    assert_eq!(value.to_string(), "12");
}

#[test]
fn run_executes_array_index_and_len() {
    let value =
        zeta::run_source(include_str!("../testdata/run_array.zeta")).expect("program should run");
    assert_eq!(value.to_string(), "9");
}

#[test]
fn run_executes_string_scan_std_core_builtins() {
    let value = zeta::run_source(include_str!("../testdata/run_string_scan.zeta"))
        .expect("program should run");
    assert_eq!(value.to_string(), "122");
}

#[test]
fn run_executes_string_build_std_core_builtins() {
    let value = zeta::run_source(include_str!("../testdata/run_string_build.zeta"))
        .expect("program should run");
    assert_eq!(value.to_string(), "score=42");
}

#[test]
fn run_executes_typed_array_builder_builtins() {
    let value = zeta::run_source(include_str!("../testdata/run_array_builder.zeta"))
        .expect("program should run");
    assert_eq!(value.to_string(), "9");
}

#[test]
fn run_executes_std_io_path_and_diagnostic_builtins() {
    let value = zeta::run_source(include_str!("../testdata/run_io_path_diagnostic.zeta"))
        .expect("program should run");
    assert_eq!(value.to_string(), "LEX_BAD_CHAR at 3:5: main.zeta");
}

#[test]
fn run_executes_std_io_file_read_to_string() {
    let path = std::env::temp_dir().join(format!("zeta-io-{}.txt", std::process::id()));
    std::fs::write(&path, "hello from zeta").expect("temp file should write");
    let source = format!(
        r#"
import std.io;

fn main() -> String {{
  let result: ResultString = file_read_to_string("{}");
  match result {{
    ResultString.Ok(text) -> {{ return text; }},
    ResultString.Err(message) -> {{ return message; }},
  }}
  return "missing";
}}
"#,
        path.display()
    );
    let value = zeta::run_source(&source).expect("program should run");
    assert_eq!(value.to_string(), "hello from zeta");
    std::fs::remove_file(path).expect("temp file should remove");
}

#[test]
fn run_executes_scalar_match() {
    let value =
        zeta::run_source(include_str!("../testdata/run_match.zeta")).expect("program should run");
    assert_eq!(value.to_string(), "42");
}

#[test]
fn run_executes_struct_field_access() {
    let value =
        zeta::run_source(include_str!("../testdata/run_struct.zeta")).expect("program should run");
    assert_eq!(value.to_string(), "42");
}

#[test]
fn run_executes_enum_variant_match() {
    let value =
        zeta::run_source(include_str!("../testdata/run_enum.zeta")).expect("program should run");
    assert_eq!(value.to_string(), "42");
}

#[test]
fn run_executes_enum_payload_match_binding() {
    let value = zeta::run_source(include_str!("../testdata/run_enum_payload.zeta"))
        .expect("program should run");
    assert_eq!(value.to_string(), "42");
}

#[test]
fn run_executes_std_core_option_and_result_int() {
    let value = zeta::run_source(include_str!("../testdata/run_std_core.zeta"))
        .expect("std.core OptionInt/ResultInt program should run");
    assert_eq!(value.to_string(), "42");
}

#[test]
fn mir_interpreter_executes_lowered_program() {
    let source = include_str!("../testdata/run_mut.zeta");
    let module = zeta::parse_source(source).expect("source should parse");
    zeta::resolver::resolve(&module).expect("source should resolve");
    zeta::typecheck::check(&module).expect("source should typecheck");

    let program = zeta::mir::lower(&module);
    let value = zeta::runtime::run_mir(&program).expect("MIR program should run");

    assert_eq!(value.to_string(), "42");
}

#[test]
fn repl_executes_scalar_match() {
    let mut session = zeta::runtime::ReplSession::new();
    let value = zeta::eval_repl_source(
        &mut session,
        r#"
fn main() -> Int {
  match 1 {
    1 -> { return 42; },
    _ -> { return 0; },
  }
  return 0;
}
"#,
    )
    .expect("REPL match program should run");

    assert_eq!(value.to_string(), "42");
}

#[test]
fn cli_runs_program() {
    let binary = env!("CARGO_BIN_EXE_zeta");
    let output = std::process::Command::new(binary)
        .args(["run", "testdata/run_basic.zeta"])
        .output()
        .expect("zeta binary should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be utf-8"),
        "42\n"
    );
}

#[test]
fn repl_evaluates_expression() {
    let binary = env!("CARGO_BIN_EXE_zeta");
    let mut child = std::process::Command::new(binary)
        .arg("repl")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("zeta repl should start");

    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().expect("stdin should be piped");
        stdin
            .write_all(b"40 + 2\n:quit\n")
            .expect("repl input should write");
    }

    let output = child.wait_with_output().expect("repl should exit");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf-8");
    assert!(stdout.contains("42"));
}

#[test]
fn run_rejects_main_with_params() {
    let diagnostics = zeta::run_source(
        r#"
fn main(name: String) -> Int {
  return 0;
}
"#,
    )
    .expect_err("main params should be rejected");

    assert_eq!(diagnostics[0].code, "RUNTIME_MAIN_PARAMS");
}

#[test]
fn run_rejects_missing_main_return() {
    let diagnostics = zeta::run_source(
        r#"
fn main() -> Int {
  let answer: Int = 42;
}
"#,
    )
    .expect_err("missing main return should be rejected");

    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "MIR_MISSING_RETURN"),
        "diagnostics: {diagnostics:?}"
    );
}
