#[test]
fn check_rejects_let_type_mismatch() {
    let source = r#"
fn main() {
  let value: Int = "not int";
}
"#;
    let diagnostics = zeta::check_source(source).expect_err("type mismatch should fail");
    assert_eq!(diagnostics[0].code, "TYPE_LET_MISMATCH");
    assert_eq!(diagnostics[0].span, span_of(source, "\"not int\""));
}

#[test]
fn check_rejects_return_type_mismatch() {
    let source = r#"
fn main() -> Int {
  return "not int";
}
"#;
    let diagnostics = zeta::check_source(source).expect_err("return mismatch should fail");
    assert_eq!(diagnostics[0].code, "TYPE_RETURN_MISMATCH");
    assert_eq!(diagnostics[0].span, span_of(source, "\"not int\""));
}

#[test]
fn check_rejects_non_bool_if_condition() {
    let source = r#"
fn main() {
  if 1 {
    return;
  }
}
"#;
    let diagnostics = zeta::check_source(source).expect_err("if condition mismatch should fail");
    assert_eq!(diagnostics[0].code, "TYPE_IF_CONDITION");
    assert_eq!(diagnostics[0].span, span_of(source, "1"));
}

#[test]
fn check_rejects_assignment_type_mismatch() {
    let source = r#"
fn main() {
  let mut value: Int = 1;
  value = "not int";
}
"#;
    let diagnostics = zeta::check_source(source).expect_err("assignment mismatch should fail");
    assert_eq!(diagnostics[0].code, "TYPE_ASSIGN_MISMATCH");
    assert_eq!(diagnostics[0].span, span_of(source, "\"not int\""));
}

#[test]
fn cli_check_renders_line_column_and_source_snippet() {
    let source = r#"
fn main() {
  let value: Int = "not int";
}
"#;
    let path = std::env::temp_dir().join(format!(
        "zeta-diagnostic-{}-{}.zeta",
        std::process::id(),
        "type-mismatch"
    ));
    std::fs::write(&path, source).expect("temp source should write");

    let binary = env!("CARGO_BIN_EXE_zeta");
    let output = std::process::Command::new(binary)
        .arg("check")
        .arg(&path)
        .output()
        .expect("zeta check should run");
    let _ = std::fs::remove_file(&path);

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf-8");
    assert!(stderr.contains("TYPE_LET_MISMATCH"));
    assert!(stderr.contains(":3:20"));
    assert!(stderr.contains("let value: Int = \"not int\";"));
    assert!(stderr.contains("^^^^^^^^^"));
}

fn span_of(source: &str, needle: &str) -> zeta::diagnostic::Span {
    let start = source.find(needle).expect("needle should exist");
    zeta::diagnostic::Span::new(start, start + needle.len())
}
