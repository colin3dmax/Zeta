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
fn check_rejects_non_int_ordering_operands() {
    let source = r#"
fn main() {
  if "a" < "b" {
    return;
  }
}
"#;
    let diagnostics = zeta::check_source(source).expect_err("ordering strings should fail");
    assert_eq!(diagnostics[0].code, "TYPE_ORDERING_OPERAND");
    assert_eq!(diagnostics[0].span, span_of(source, "\"a\""));
}

#[test]
fn check_rejects_non_bool_logical_operands() {
    let source = r#"
fn main() {
  if 1 && true {
    return;
  }
}
"#;
    let diagnostics = zeta::check_source(source).expect_err("logical Int should fail");
    assert_eq!(diagnostics[0].code, "TYPE_LOGICAL_OPERAND");
    assert_eq!(diagnostics[0].span, span_of(source, "1"));
}

#[test]
fn check_rejects_non_bool_not_operand() {
    let source = r#"
fn main() {
  if !1 {
    return;
  }
}
"#;
    let diagnostics = zeta::check_source(source).expect_err("not Int should fail");
    assert_eq!(diagnostics[0].code, "TYPE_UNARY_OPERAND");
    assert_eq!(diagnostics[0].span, span_of(source, "1"));
}

#[test]
fn check_rejects_match_pattern_type_mismatch() {
    let source = r#"
fn main() {
  match true {
    1 -> { return; },
    _ -> { return; },
  }
}
"#;
    let diagnostics = zeta::check_source(source).expect_err("Int pattern against Bool should fail");
    assert_eq!(diagnostics[0].code, "TYPE_MATCH_PATTERN");
    assert_eq!(diagnostics[0].span, span_of(source, "true"));
}

#[test]
fn check_rejects_struct_field_type_mismatch() {
    let source = r#"
struct User {
  name: String,
  age: Int,
}

fn main() -> Int {
  let user: User = User { name: "Ada", age: "old" };
  return user.age;
}
"#;
    let diagnostics =
        zeta::check_source(source).expect_err("String field value for Int field should fail");
    assert_eq!(diagnostics[0].code, "TYPE_STRUCT_FIELD");
    assert_eq!(diagnostics[0].span, span_of(source, "\"old\""));
}

#[test]
fn check_rejects_unknown_enum_variant() {
    let source = r#"
enum ResultTag {
  Ok,
  Err,
}

fn main() -> Int {
  let tag: ResultTag = ResultTag.Missing;
  return 0;
}
"#;
    let diagnostics = zeta::check_source(source).expect_err("unknown variant should fail");
    assert_eq!(diagnostics[0].code, "TYPE_UNKNOWN_VARIANT");
    assert_eq!(diagnostics[0].span, span_of(source, "Missing"));
}

#[test]
fn check_rejects_enum_payload_type_mismatch() {
    let source = r#"
enum OptionInt {
  Some(Int),
  None,
}

fn main() -> Int {
  let value: OptionInt = OptionInt.Some("not int");
  return 0;
}
"#;
    let diagnostics = zeta::check_source(source).expect_err("payload mismatch should fail");
    assert_eq!(diagnostics[0].code, "TYPE_ENUM_VARIANT_PAYLOAD");
    assert_eq!(diagnostics[0].span, span_of(source, "\"not int\""));
}

#[test]
fn check_rejects_enum_payload_pattern_without_binding() {
    let source = r#"
enum OptionInt {
  Some(Int),
  None,
}

fn main() -> Int {
  let value: OptionInt = OptionInt.Some(42);
  match value {
    OptionInt.Some -> { return 1; },
    OptionInt.None -> { return 0; },
  }
  return 0;
}
"#;
    let diagnostics = zeta::check_source(source).expect_err("missing binding should fail");
    assert_eq!(diagnostics[0].code, "TYPE_ENUM_PATTERN_ARITY");
}

#[test]
fn check_rejects_non_exhaustive_bool_match() {
    let source = r#"
fn main() -> Int {
  match true {
    true -> { return 42; },
  }
  return 0;
}
"#;
    let diagnostics = zeta::check_source(source).expect_err("partial Bool match should fail");
    assert_eq!(diagnostics[0].code, "TYPE_MATCH_NON_EXHAUSTIVE");
    assert_eq!(diagnostics[0].span, span_of(source, "true"));
}

#[test]
fn check_rejects_non_exhaustive_enum_match() {
    let source = r#"
enum ResultTag {
  Ok,
  Err,
}

fn main() -> Int {
  let tag: ResultTag = ResultTag.Ok;
  match tag {
    ResultTag.Ok -> { return 42; },
  }
  return 0;
}
"#;
    let diagnostics = zeta::check_source(source).expect_err("partial enum match should fail");
    assert_eq!(diagnostics[0].code, "TYPE_MATCH_NON_EXHAUSTIVE");
    let match_tag_start = source.find("match tag").expect("match should exist") + "match ".len();
    assert_eq!(
        diagnostics[0].span,
        zeta::diagnostic::Span::new(match_tag_start, match_tag_start + "tag".len())
    );
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
