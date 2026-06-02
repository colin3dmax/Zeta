#[test]
fn module_graph_accepts_local_imports() {
    let files = vec![
        source_file(
            "app.zeta",
            r#"
module demo.app;
import demo.math;

fn main() -> Int {
  return answer();
}
"#,
        ),
        source_file(
            "math.zeta",
            r#"
module demo.math;

export fn answer() -> Int {
  return 42;
}
"#,
        ),
    ];

    zeta::module_graph::check_sources(&files).expect("local module import should resolve");
}

#[test]
fn module_graph_accepts_import_alias_qualified_calls() {
    let files = vec![
        source_file(
            "app.zeta",
            r#"
module demo.app;
import demo.math as math;

fn main() -> Int {
  return math.answer();
}
"#,
        ),
        source_file(
            "math.zeta",
            r#"
module demo.math;

export fn answer() -> Int {
  return 42;
}
"#,
        ),
    ];

    zeta::module_graph::check_sources(&files).expect("import alias should resolve");
}

#[test]
fn module_graph_accepts_reexported_functions() {
    let files = vec![
        source_file(
            "app.zeta",
            r#"
module demo.app;
import demo.facade;

fn main() -> Int {
  return answer();
}
"#,
        ),
        source_file(
            "facade.zeta",
            r#"
module demo.facade;
export import demo.math;
"#,
        ),
        source_file(
            "math.zeta",
            r#"
module demo.math;

export fn answer() -> Int {
  return 42;
}
"#,
        ),
    ];

    let value = zeta::module_graph::run_sources(&files).expect("re-exported call should run");
    assert_eq!(value.to_string(), "42");
}

#[test]
fn module_graph_accepts_qualified_reexported_function_calls() {
    let files = vec![
        source_file(
            "app.zeta",
            r#"
module demo.app;
import demo.facade;

fn main() -> Int {
  return demo.facade.answer();
}
"#,
        ),
        source_file(
            "facade.zeta",
            r#"
module demo.facade;
export import demo.math;
"#,
        ),
        source_file(
            "math.zeta",
            r#"
module demo.math;

export fn answer() -> Int {
  return 42;
}
"#,
        ),
    ];

    let value =
        zeta::module_graph::run_sources(&files).expect("qualified re-exported call should run");
    assert_eq!(value.to_string(), "42");
}

#[test]
fn module_graph_accepts_imported_struct_field_access() {
    let files = vec![
        source_file(
            "app.zeta",
            r#"
module demo.app;
import demo.model;

fn main() -> Int {
  let user: User = make_user();
  return user.age;
}
"#,
        ),
        source_file(
            "model.zeta",
            r#"
module demo.model;

export struct User {
  name: String,
  age: Int,
}

export fn make_user() -> User {
  return User { name: "Ada", age: 42 };
}
"#,
        ),
    ];

    let value = zeta::module_graph::run_sources(&files)
        .expect("imported exported struct fields should typecheck and run");
    assert_eq!(value.to_string(), "42");
}

#[test]
fn module_graph_accepts_imported_enum_payload_match() {
    let files = vec![
        source_file(
            "app.zeta",
            r#"
module demo.app;
import demo.model;

fn main() -> Int {
  let result: ResultInt = answer();
  match result {
    ResultInt.Ok(value) -> { return value; },
    ResultInt.Err(message) -> { return 0; },
  }
  return 0;
}
"#,
        ),
        source_file(
            "model.zeta",
            r#"
module demo.model;

export enum ResultInt {
  Ok(Int),
  Err(String),
}

export fn answer() -> ResultInt {
  return ResultInt.Ok(42);
}
"#,
        ),
    ];

    let value = zeta::module_graph::run_sources(&files)
        .expect("imported exported enum payload match should typecheck and run");
    assert_eq!(value.to_string(), "42");
}

#[test]
fn module_graph_rejects_ambiguous_reexports() {
    let files = vec![
        source_file(
            "app.zeta",
            r#"
module demo.app;
import demo.facade;

fn main() -> Int {
  return answer();
}
"#,
        ),
        source_file(
            "facade.zeta",
            r#"
module demo.facade;
export import demo.math;
export import demo.more;
"#,
        ),
        source_file(
            "math.zeta",
            r#"
module demo.math;

export fn answer() -> Int {
  return 40;
}
"#,
        ),
        source_file(
            "more.zeta",
            r#"
module demo.more;

export fn answer() -> Int {
  return 2;
}
"#,
        ),
    ];

    let errors =
        zeta::module_graph::check_sources(&files).expect_err("ambiguous re-export should fail");
    assert_eq!(errors[0].diagnostics[0].code, "RESOLVE_AMBIGUOUS_REEXPORT");
}

#[test]
fn module_graph_accepts_duplicate_imports_of_same_function_origin() {
    let files = vec![
        source_file(
            "app.zeta",
            r#"
module demo.app;
import demo.math;
import demo.math as math;

fn main() -> Int {
  return answer();
}
"#,
        ),
        source_file(
            "math.zeta",
            r#"
module demo.math;

export fn answer() -> Int {
  return 42;
}
"#,
        ),
    ];

    zeta::module_graph::check_sources(&files)
        .expect("duplicate imports of one module should not make short calls ambiguous");
}

#[test]
fn module_graph_rejects_ambiguous_imported_short_function_call() {
    let files = vec![
        source_file(
            "app.zeta",
            r#"
module demo.app;
import demo.math;
import demo.more;

fn main() -> Int {
  return answer();
}
"#,
        ),
        source_file(
            "math.zeta",
            r#"
module demo.math;

export fn answer() -> Int {
  return 40;
}
"#,
        ),
        source_file(
            "more.zeta",
            r#"
module demo.more;

export fn answer() -> Int {
  return 2;
}
"#,
        ),
    ];

    let errors =
        zeta::module_graph::check_sources(&files).expect_err("ambiguous short call should fail");
    assert_eq!(errors[0].diagnostics[0].code, "RESOLVE_AMBIGUOUS_FUNCTION");
    assert_eq!(
        errors[0].diagnostics[0].span,
        span_of(files[0].source.as_str(), "answer")
    );
}

#[test]
fn module_graph_accepts_qualified_calls_when_short_name_is_ambiguous() {
    let files = vec![
        source_file(
            "app.zeta",
            r#"
module demo.app;
import demo.math;
import demo.more as more;

fn main() -> Int {
  return demo.math.answer() + more.answer();
}
"#,
        ),
        source_file(
            "math.zeta",
            r#"
module demo.math;

export fn answer() -> Int {
  return 40;
}
"#,
        ),
        source_file(
            "more.zeta",
            r#"
module demo.more;

export fn answer() -> Int {
  return 2;
}
"#,
        ),
    ];

    let value = zeta::module_graph::run_sources(&files).expect("qualified calls should run");
    assert_eq!(value.to_string(), "42");
}

#[test]
fn module_graph_does_not_import_private_functions() {
    let files = vec![
        source_file(
            "app.zeta",
            r#"
module demo.app;
import demo.math;

fn main() -> Int {
  return answer();
}
"#,
        ),
        source_file(
            "math.zeta",
            r#"
module demo.math;

fn answer() -> Int {
  return 42;
}
"#,
        ),
    ];

    let errors =
        zeta::module_graph::check_sources(&files).expect_err("private function should not import");
    assert_eq!(errors[0].diagnostics[0].code, "RESOLVE_UNKNOWN_FUNCTION");
}

#[test]
fn module_graph_rejects_missing_local_imports() {
    let source = r#"
module demo.app;
import demo.missing;

fn main() -> Int {
  return 42;
}
"#;
    let files = vec![source_file("app.zeta", source)];
    let errors = zeta::module_graph::check_sources(&files).expect_err("missing import should fail");
    assert_eq!(errors[0].diagnostics[0].code, "RESOLVE_UNKNOWN_IMPORT");
    assert_eq!(
        errors[0].diagnostics[0].span,
        span_of(source, "demo.missing")
    );
}

#[test]
fn module_graph_rejects_duplicate_modules() {
    let files = vec![
        source_file(
            "one.zeta",
            "module demo.same;\nfn one() -> Int { return 1; }\n",
        ),
        source_file(
            "two.zeta",
            "module demo.same;\nfn two() -> Int { return 2; }\n",
        ),
    ];

    let errors =
        zeta::module_graph::check_sources(&files).expect_err("duplicate module should fail");
    assert_eq!(errors[0].diagnostics[0].code, "RESOLVE_DUPLICATE_MODULE");
}

#[test]
fn cli_check_accepts_module_directory() {
    let binary = env!("CARGO_BIN_EXE_zeta");
    let output = std::process::Command::new(binary)
        .args(["check", "testdata/modules_ok"])
        .output()
        .expect("zeta check should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be utf-8"),
        "ok\n"
    );
}

#[test]
fn cli_run_executes_module_directory() {
    let binary = env!("CARGO_BIN_EXE_zeta");
    let output = std::process::Command::new(binary)
        .args(["run", "testdata/modules_ok"])
        .output()
        .expect("zeta run should run");

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
fn cli_run_executes_qualified_module_call() {
    let binary = env!("CARGO_BIN_EXE_zeta");
    let output = std::process::Command::new(binary)
        .args(["run", "testdata/modules_qualified"])
        .output()
        .expect("zeta run should run");

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
fn cli_run_executes_import_alias_module_call() {
    let binary = env!("CARGO_BIN_EXE_zeta");
    let output = std::process::Command::new(binary)
        .args(["run", "testdata/modules_alias"])
        .output()
        .expect("zeta run should run");

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

fn source_file(path: &str, source: &str) -> zeta::module_graph::SourceFile {
    zeta::module_graph::SourceFile {
        path: path.to_string(),
        source: source.to_string(),
    }
}

fn span_of(source: &str, needle: &str) -> zeta::diagnostic::Span {
    let start = source.find(needle).expect("needle should exist");
    zeta::diagnostic::Span::new(start, start + needle.len())
}
