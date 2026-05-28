#[test]
fn check_rejects_let_type_mismatch() {
    let source = r#"
fn main() {
  let value: Int = "not int";
}
"#;
    let diagnostics = zeta::check_source(source).expect_err("type mismatch should fail");
    assert_eq!(diagnostics[0].code, "TYPE_LET_MISMATCH");
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
}
