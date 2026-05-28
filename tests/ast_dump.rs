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
    assert!(stdout.contains("Zeta REPL prototype"));
    assert!(stdout.contains("Function name=repl exported=false"));
    assert!(stdout.contains("Let name=answer type=Int"));
    assert!(stdout.contains("Binary op=add"));
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
            .write_all(b":help\n:doc let\n:complete st\n:quit\n")
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
    assert!(stdout.contains("struct"));
}
