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

fn span_of(source: &str, needle: &str) -> zeta::diagnostic::Span {
    let start = source.find(needle).expect("needle should exist");
    zeta::diagnostic::Span::new(start, start + needle.len())
}
