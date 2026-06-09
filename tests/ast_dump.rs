#[test]
fn dumps_core_items() {
    let source = include_str!("../testdata/core_items.zeta");
    let expected = include_str!("../testdata/core_items.ast");
    let dump = zeta::dump_ast(source).expect("source should parse");
    assert_eq!(dump, expected);
}

#[test]
fn cli_dumps_core_items() {
    let binary = env!("CARGO_BIN_EXE_zeta");
    let output = std::process::Command::new(binary)
        .args(["ast-dump", "testdata/core_items.zeta"])
        .output()
        .expect("zeta binary should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be utf-8"),
        include_str!("../testdata/core_items.ast")
    );
}

#[test]
fn parses_dotted_module_declaration() {
    let dump = zeta::dump_ast("module demo.core;\n").expect("source should parse");
    assert_eq!(dump, "Module\n  ModuleDecl name=demo.core\n");
}

#[test]
fn dumps_import_alias() {
    let dump = zeta::dump_ast("import demo.math as math;\n").expect("source should parse");
    assert_eq!(dump, "Module\n  Import path=demo.math alias=math\n");
}

#[test]
fn dumps_export_import() {
    let dump = zeta::dump_ast("export import demo.math;\n").expect("source should parse");
    assert_eq!(dump, "Module\n  Import path=demo.math exported=true\n");
}

#[test]
fn dumps_mutable_binding_and_assignment() {
    let dump = zeta::dump_ast(
        r#"
fn main() {
  let mut value: Int = 1;
  value = value + 1;
}
"#,
    )
    .expect("source should parse");
    assert_eq!(
        dump,
        "Module\n  Function name=main exported=false\n    Let name=value type=Int mutable=true\n      Int 1\n    Assign name=value\n      Binary op=add\n        Name value\n        Int 1\n"
    );
}

#[test]
fn dumps_comparison_expressions() {
    let dump = zeta::dump_ast(
        r#"
fn main() -> Bool {
  return 1 + 1 == 2;
}
"#,
    )
    .expect("source should parse");

    assert!(dump.contains("Binary op=eq"));
    assert!(dump.contains("Binary op=add"));
}

#[test]
fn dumps_boolean_logic_expressions() {
    let dump = zeta::dump_ast(
        r#"
fn main() -> Bool {
  return true && !false || false;
}
"#,
    )
    .expect("source should parse");

    assert!(dump.contains("Binary op=or"));
    assert!(dump.contains("Binary op=and"));
    assert!(dump.contains("Unary op=not"));
}

#[test]
fn dumps_unary_negation() {
    let dump = zeta::dump_ast(
        r#"
fn main() -> Int {
  return -value - -1;
}
"#,
    )
    .expect("source should parse");

    assert!(dump.contains("Unary op=neg"));
    assert!(dump.contains("Binary op=sub"));
}

#[test]
fn repl_parses_interactive_lines() {
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
            .write_all(b"let answer: Int = 40 + 2;\n:quit\n")
            .expect("repl input should write");
    }

    let output = child.wait_with_output().expect("repl should exit");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf-8");
    assert!(stdout.contains("Stage 0 language shell"));
    assert!(stdout.contains("ok"));
}

#[test]
fn repl_reports_unsupported_float_literal() {
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
            .write_all(b"1./3\n:quit\n")
            .expect("repl input should write");
    }

    let output = child.wait_with_output().expect("repl should exit");
    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf-8");
    assert!(stderr.contains("LEX_FLOAT_UNSUPPORTED"));
    assert!(!stderr.contains("PARSE_EXPECTED_ITEM"));
}

#[test]
fn repl_supports_help_doc_and_completion() {
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
            .write_all(b":help\n:doc let\n:doc api\n:complete st\n:quit\n")
            .expect("repl input should write");
    }

    let output = child.wait_with_output().expect("repl should exit");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout should be utf-8");
    assert!(stdout.contains(":doc <topic>"));
    assert!(stdout.contains("Declare a local binding"));
    assert!(stdout.contains("standard API surface"));
    assert!(stdout.contains("struct"));
    assert_eq!(zeta::repl::complete("std."), vec!["std.core", "std.io"]);
    assert!(zeta::repl::result_line("8").contains("=>"));
}

#[test]
fn repl_highlight_uses_color_ansi_codes() {
    let highlighted = zeta::repl::highlight_zeta(":help let answer: Int = 40;");

    assert!(highlighted.contains("\x1b[1;38;5;214m:help\x1b[0m"));
    assert!(highlighted.contains("\x1b[1;38;5;81mlet\x1b[0m"));
    assert!(highlighted.contains("\x1b[1;38;5;141mInt\x1b[0m"));
}
