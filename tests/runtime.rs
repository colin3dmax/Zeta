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
