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
fn module_graph_accepts_reexported_structs() {
    let files = vec![
        source_file(
            "app.zeta",
            r#"
module demo.app;
import demo.facade;

fn main() -> Int {
  let user: User = make_user();
  return user.age;
}
"#,
        ),
        source_file(
            "facade.zeta",
            r#"
module demo.facade;
export import demo.model;
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

    let value = zeta::module_graph::run_sources(&files).expect("re-exported struct should run");
    assert_eq!(value.to_string(), "42");
}

#[test]
fn module_graph_accepts_reexported_enums() {
    let files = vec![
        source_file(
            "app.zeta",
            r#"
module demo.app;
import demo.facade;

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
            "facade.zeta",
            r#"
module demo.facade;
export import demo.model;
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

    let value = zeta::module_graph::run_sources(&files).expect("re-exported enum should run");
    assert_eq!(value.to_string(), "42");
}

#[test]
fn module_graph_dumps_stable_reexported_symbols() {
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
export import demo.model;
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

export enum ResultInt {
  Ok(Int),
  Err(String),
}

export fn answer() -> Int {
  return 42;
}
"#,
        ),
    ];

    let dump = zeta::module_graph::dump_symbols(&files).expect("symbols should dump");
    assert_eq!(
        dump,
        "ModuleSymbols\n  module demo.app\n  module demo.facade\n    fn answer symbol=demo.model.answer params=(Unit) return=Int\n    struct User symbol=demo.model.User fields=age:Int,name:String\n    enum ResultInt symbol=demo.model.ResultInt variants=Err(String),Ok(Int)\n  module demo.model\n    fn answer symbol=demo.model.answer params=(Unit) return=Int\n    struct User symbol=demo.model.User fields=age:Int,name:String\n    enum ResultInt symbol=demo.model.ResultInt variants=Err(String),Ok(Int)\n"
    );
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
fn module_graph_accepts_imported_enum_variant_construction() {
    let files = vec![
        source_file(
            "app.zeta",
            r#"
module demo.app;
import demo.model;

fn main() -> Int {
  let result: ResultInt = ResultInt.Ok(42);
  return unwrap(result);
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

export fn unwrap(result: ResultInt) -> Int {
  match result {
    ResultInt.Ok(value) -> { return value; },
    ResultInt.Err(message) -> { return 0; },
  }
  return 0;
}
"#,
        ),
    ];

    let value = zeta::module_graph::run_sources(&files)
        .expect("imported enum variant construction should typecheck and run");
    assert_eq!(value.to_string(), "42");
}

#[test]
fn module_graph_accepts_std_core_option_int() {
    let files = vec![source_file(
        "app.zeta",
        r#"
module demo.app;
import std.core;

fn main() -> Int {
  let value: OptionInt = OptionInt.None;
  match value {
    OptionInt.Some(answer) -> { return answer; },
    OptionInt.None -> { return 42; },
  }
  return 0;
}
"#,
    )];

    let value = zeta::module_graph::run_sources(&files)
        .expect("std.core OptionInt should typecheck and run in module graph mode");
    assert_eq!(value.to_string(), "42");
}

#[test]
fn module_graph_rejects_ambiguous_imported_type_names() {
    let files = vec![
        source_file(
            "app.zeta",
            r#"
module demo.app;
import demo.alpha;
import demo.beta;

fn main() -> Int {
  return 42;
}
"#,
        ),
        source_file(
            "alpha.zeta",
            r#"
module demo.alpha;

export struct Item {
  value: Int,
}
"#,
        ),
        source_file(
            "beta.zeta",
            r#"
module demo.beta;

export enum Item {
  Found(Int),
  Missing,
}
"#,
        ),
    ];

    let errors =
        zeta::module_graph::check_sources(&files).expect_err("ambiguous type import should fail");
    assert_eq!(errors[0].diagnostics[0].code, "RESOLVE_AMBIGUOUS_TYPE");
}

#[test]
fn module_graph_rejects_ambiguous_imported_struct_names() {
    let files = vec![
        source_file(
            "app.zeta",
            r#"
module demo.app;
import demo.alpha;
import demo.beta;

fn main() -> Int {
  let item: Item = make_alpha();
  return item.value;
}
"#,
        ),
        source_file(
            "alpha.zeta",
            r#"
module demo.alpha;

export struct Item {
  value: Int,
}

export fn make_alpha() -> Item {
  return Item { value: 40 };
}
"#,
        ),
        source_file(
            "beta.zeta",
            r#"
module demo.beta;

export struct Item {
  value: Int,
}
"#,
        ),
    ];

    let errors =
        zeta::module_graph::check_sources(&files).expect_err("ambiguous struct import should fail");
    assert_eq!(errors[0].diagnostics[0].code, "RESOLVE_AMBIGUOUS_TYPE");
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
fn module_graph_rejects_ambiguous_type_reexports() {
    let files = vec![
        source_file(
            "app.zeta",
            r#"
module demo.app;
import demo.facade;

fn main() -> Int {
  return 42;
}
"#,
        ),
        source_file(
            "facade.zeta",
            r#"
module demo.facade;
export import demo.alpha;
export import demo.beta;
"#,
        ),
        source_file(
            "alpha.zeta",
            r#"
module demo.alpha;

export struct Item {
  value: Int,
}
"#,
        ),
        source_file(
            "beta.zeta",
            r#"
module demo.beta;

export enum Item {
  Found(Int),
  Missing,
}
"#,
        ),
    ];

    let errors = zeta::module_graph::check_sources(&files)
        .expect_err("ambiguous type re-export should fail");
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
