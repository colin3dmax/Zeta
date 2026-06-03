#[test]
fn check_accepts_core_items() {
    let source = include_str!("../testdata/core_items.zeta");
    zeta::check_source(source).expect("core corpus should resolve");
}

#[test]
fn check_rejects_duplicate_items() {
    let source = r#"
fn main() {
  return 0;
}

struct main {
  value: Int,
}
"#;
    let diagnostics = zeta::check_source(source).expect_err("duplicate item should fail");
    assert_eq!(diagnostics[0].code, "RESOLVE_DUPLICATE_ITEM");
}

#[test]
fn check_rejects_duplicate_locals() {
    let source = r#"
fn main(value: Int) {
  let value: Int = 1;
}
"#;
    let diagnostics = zeta::check_source(source).expect_err("duplicate local should fail");
    assert_eq!(diagnostics[0].code, "RESOLVE_DUPLICATE_LOCAL");
}

#[test]
fn check_rejects_unknown_names() {
    let source = r#"
fn main() {
  return missing;
}
"#;
    let diagnostics = zeta::check_source(source).expect_err("unknown name should fail");
    assert_eq!(diagnostics[0].code, "RESOLVE_UNKNOWN_NAME");
    assert_eq!(diagnostics[0].span, span_of(source, "missing"));
}

#[test]
fn check_let_initializer_cannot_reference_declared_name() {
    let source = r#"
fn main() {
  let value: Int = value + 1;
}
"#;
    let diagnostics = zeta::check_source(source).expect_err("self reference should fail");
    assert_eq!(diagnostics[0].code, "RESOLVE_UNKNOWN_NAME");
}

#[test]
fn check_rejects_assignment_to_immutable_local() {
    let source = r#"
fn main() {
  let value: Int = 1;
  value = 2;
}
"#;
    let diagnostics = zeta::check_source(source).expect_err("immutable assignment should fail");
    assert_eq!(diagnostics[0].code, "RESOLVE_ASSIGN_IMMUTABLE");
    let assignment = source.find("value = 2").expect("assignment should exist");
    assert_eq!(
        diagnostics[0].span,
        zeta::diagnostic::Span::new(assignment, assignment + "value".len())
    );
}

#[test]
fn check_accepts_assignment_to_mutable_local() {
    let source = r#"
fn main() {
  let mut value: Int = 1;
  value = value + 1;
}
"#;
    zeta::check_source(source).expect("mutable assignment should resolve");
}

#[test]
fn check_accepts_stage0_standard_imports() {
    let source = r#"
import std.core;
import std.io;

fn main() -> Int {
  return 42;
}
"#;
    zeta::check_source(source).expect("standard imports should resolve");
}

#[test]
fn check_rejects_std_core_type_shadowing() {
    let source = r#"
import std.core;

enum OptionInt {
  Local,
}

fn main() {
  return;
}
"#;
    let diagnostics =
        zeta::check_source(source).expect_err("local item should not shadow std.core type");
    assert_eq!(diagnostics[0].code, "RESOLVE_DUPLICATE_ITEM");
    assert_eq!(diagnostics[0].span, span_of(source, "OptionInt"));
}

#[test]
fn check_rejects_unknown_imports() {
    let source = r#"
import std.net;

fn main() -> Int {
  return 42;
}
"#;
    let diagnostics = zeta::check_source(source).expect_err("unknown import should fail");
    assert_eq!(diagnostics[0].code, "RESOLVE_UNKNOWN_IMPORT");
    assert_eq!(diagnostics[0].span, span_of(source, "std.net"));
}

fn span_of(source: &str, needle: &str) -> zeta::diagnostic::Span {
    let start = source.find(needle).expect("needle should exist");
    zeta::diagnostic::Span::new(start, start + needle.len())
}
