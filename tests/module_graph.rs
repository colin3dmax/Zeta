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
fn module_graph_accepts_imported_types_in_signatures_and_locals() {
    let files = vec![
        source_file(
            "app.zeta",
            r#"
module demo.app;
import demo.model;

fn age(user: User) -> Int {
  return user.age;
}

fn main() -> Int {
  let user: User = make_user();
  return age(user);
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
        .expect("imported types should be accepted in signatures and locals");
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

#[test]
fn cli_run_executes_stage1_frontend_seed() {
    let binary = env!("CARGO_BIN_EXE_zeta");
    let output = std::process::Command::new(binary)
        .args(["run", "testdata/stage1_frontend"])
        .output()
        .expect("zeta run should run");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be utf-8"),
        "111\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reuses_stage1_frontend_contract() {
    let stage2_app = r#"
module stage2.app;
import std.io;
import stage1.frontend;

fn main() -> Int {
  let result: ResultString = file_read_to_string("testdata/stage2_bootstrap/input.zeta");
  match result {
    ResultString.Ok(source) -> { return stage1.frontend.ast_dump_score(source); },
    ResultString.Err(message) -> { return 0; },
  }
  return 0;
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/main.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap harness should run");

    assert_eq!(value.to_string(), "111");
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_text_summary() {
    let stage2_app = r#"
module stage2.summary;
import std.io;
import stage1.frontend;

fn main() -> String {
  let result: ResultString = file_read_to_string("testdata/stage2_bootstrap/input.zeta");
  match result {
    ResultString.Ok(source) -> { return stage1.frontend.ast_dump_summary(source); },
    ResultString.Err(message) -> { return message; },
  }
  return "missing";
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/summary.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap summary harness should run");

    assert_eq!(value.to_string(), "fn=1;let=1;return=1");
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_keyword_summary() {
    let stage2_app = r#"
module stage2.keywords;
import stage1.frontend;

fn main() -> String {
  let source: String = "module demo.app; import demo.math as math; export struct Box { value: Int } enum Flag { On, Off } fn main() -> Int { let mut x: Int = 0; while x <= 1 { x = x + 1; continue; } if x >= 1 { match Flag.On { Flag.On -> { return x; }, Flag.Off -> { break; } } } else { return 0; } return 0; }";
  return stage1.frontend.ast_dump_keyword_summary(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/keywords.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap keyword summary harness should run");

    assert_eq!(
        value.to_string(),
        "module=1;import=1;as=1;export=1;fn=1;let=1;mut=1;return=3;break=1;continue=1;if=1;else=1;while=1;match=1;struct=1;enum=1"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_symbol_summary() {
    let stage2_app = r#"
module stage2.symbols;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main(items: IntArray) -> Int { let x: Int = items[0]; if !(x == 1) && x != 2 || x < 3 || x <= 4 || x > 5 || x >= 6 { return (x + 1) - 2 * 3 / 4; } return demo.math.value; }";
  return stage1.frontend.ast_dump_symbol_summary(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/symbols.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap symbol summary harness should run");

    assert_eq!(
        value.to_string(),
        "lparen=3;rparen=3;lbrace=2;rbrace=2;lbracket=1;rbracket=1;colon=2;semicolon=3;comma=0;dot=2;arrow=1;eq=1;eqeq=1;bang=1;bangeq=1;andand=1;oror=4;lt=1;lte=1;gt=1;gte=1;plus=1;minus=1;star=1;slash=1"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_token_class_summary() {
    let stage2_app = r#"
module stage2.token_classes;
import stage1.frontend;

fn main() -> String {
  let source: String = "alpha _beta value_2 123 456 \"hello\" \"world\" true false @";
  return stage1.frontend.ast_dump_token_class_summary(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/token_classes.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap token class summary harness should run");

    assert_eq!(
        value.to_string(),
        "ident=3;int=2;string=2;bool=2;unknown=1;eof=1"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_item_summary() {
    let stage2_app = r#"
module stage2.items;
import stage1.frontend;

fn main() -> String {
  let source: String = "module demo.app; import demo.math as math; export import demo.extra; export struct Box { value: Int } enum Flag { On, Off } export fn main() -> Int { let nested: Int = 1; if nested == 1 { return nested; } return 0; } fn helper() -> Int { return 1; }";
  return stage1.frontend.ast_dump_item_summary(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/items.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap item summary harness should run");

    assert_eq!(value.to_string(), "module=1;import=2;struct=1;enum=1;fn=2");
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_export_summary() {
    let stage2_app = r#"
module stage2.exports;
import stage1.frontend;

fn main() -> String {
  let source: String = "module demo.app; export import demo.extra; import demo.math; export struct Box { value: Int } export enum Flag { On, Off } export fn main() -> Int { return 0; } fn helper() -> Int { let export_name: Int = 1; return export_name; }";
  return stage1.frontend.ast_dump_export_summary(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/exports.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap export summary harness should run");

    assert_eq!(
        value.to_string(),
        "export_import=1;export_struct=1;export_enum=1;export_fn=1"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_import_summary() {
    let stage2_app = r#"
module stage2.imports;
import stage1.frontend;

fn main() -> String {
  let source: String = "module demo.app; import demo.math; import demo.text.format as fmt; export import demo.extra.tools; fn main() -> Int { let noise: String = \"import fake.path as fake;\"; return 0; }";
  return stage1.frontend.ast_dump_import_summary(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/imports.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap import summary harness should run");

    assert_eq!(
        value.to_string(),
        "imports=3;aliased=1;exported=1;path_segments=8"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_item_dump() {
    let stage2_app = r#"
module stage2.item_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "module demo.app; import demo.math; import demo.text.format as fmt; export import demo.extra.tools; struct Box { value: Int } export enum Flag { On, Off } export fn main() -> Int { let nested: Int = 1; if nested == 1 { return nested; } return 0; } fn helper() -> Int { return 1; }";
  return stage1.frontend.ast_dump_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/item_dump.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap item dump harness should run");

    assert_eq!(
        value.to_string(),
        "items\n  module\n  import segments=2 alias=false exported=false\n  import segments=3 alias=true exported=false\n  import segments=3 alias=false exported=true\n  struct exported=false\n  enum exported=true\n  fn exported=true\n  fn exported=false\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_named_item_dump() {
    let stage2_app = r#"
module stage2.named_item_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "module demo.app; import demo.math; import demo.text.format as fmt; export import demo.extra.tools; struct Box { value: Int } export enum Flag { On, Off } export fn main() -> Int { let nested: Int = 1; if nested == 1 { return nested; } return 0; } fn helper() -> Int { return 1; }";
  return stage1.frontend.ast_dump_named_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/named_item_dump.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap named item dump harness should run");

    assert_eq!(
        value.to_string(),
        "items\n  module name=demo.app\n  import path=demo.math exported=false\n  import path=demo.text.format alias=fmt exported=false\n  import path=demo.extra.tools exported=true\n  struct name=Box exported=false\n  enum name=Flag exported=true\n  fn name=main exported=true\n  fn name=helper exported=false\n"
    );
}

#[test]
fn stage2_bootstrap_harness_item_import_export_summaries_ignore_noise() {
    let stage2_app = r#"
module stage2.item_noise;
import std.core;
import stage1.frontend;

fn main() -> String {
  let source: String = "module demo.app; import demo.math; export fn main() -> Int { let text: String = \"export import fake.path; { fn hidden() }\"; // export struct Noise { value: Int }\n return 0; }";
  let items: String = stage1.frontend.ast_dump_item_summary(source);
  let exports: String = stage1.frontend.ast_dump_export_summary(source);
  let imports: String = stage1.frontend.ast_dump_import_summary(source);
  return string_concat(string_concat(string_concat(string_concat(items, "|"), exports), "|"), imports);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/item_noise.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap item noise harness should run");

    assert_eq!(
        value.to_string(),
        "module=1;import=1;struct=0;enum=0;fn=1|export_import=0;export_struct=0;export_enum=0;export_fn=1|imports=1;aliased=0;exported=0;path_segments=2"
    );
}

#[test]
fn stage2_bootstrap_harness_named_item_dump_ignores_text_noise() {
    let stage2_app = r#"
module stage2.named_item_noise;
import stage1.frontend;

fn main() -> String {
  let source: String = "module demo.clean; import demo.real as real; export fn main() -> Int { let text: String = \"module fake.noise; import fake.path as nope; export fn hidden()\"; // export struct Noise { value: Int }\n return 0; }";
  return stage1.frontend.ast_dump_named_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/named_item_noise.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap named item noise harness should run");

    assert_eq!(
        value.to_string(),
        "items\n  module name=demo.clean\n  import path=demo.real alias=real exported=false\n  fn name=main exported=true\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_signature_dump() {
    let stage2_app = r#"
module stage2.signature_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "export struct User { id: Int, name: String } enum Result { Ok(Int), Err(String), None } export fn add(a: Int, b: String) -> Bool { other(x: Int); return true; } fn ping() { return; }";
  return stage1.frontend.ast_dump_signature_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/signature_dump.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap signature dump harness should run");

    assert_eq!(
        value.to_string(),
        "signatures\n  struct name=User exported=true\n    field name=id type=Int\n    field name=name type=String\n  enum name=Result exported=false\n    variant name=Ok payload=Int\n    variant name=Err payload=String\n    variant name=None\n  fn name=add exported=true\n    param name=a type=Int\n    param name=b type=String\n    return type=Bool\n  fn name=ping exported=false\n    return type=Unit\n"
    );
}

#[test]
fn stage2_bootstrap_harness_signature_dump_ignores_text_noise() {
    let stage2_app = r#"
module stage2.signature_noise;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main(a: Int) -> Int { let text: String = \"struct Fake { id: Int } enum Noise { Bad(String) } fn nope(x: Int) -> Bool\"; // fn comment(a: String) -> Bool\n return a; }";
  return stage1.frontend.ast_dump_signature_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/signature_noise.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap signature noise harness should run");

    assert_eq!(
        value.to_string(),
        "signatures\n  fn name=main exported=false\n    param name=a type=Int\n    return type=Int\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_item_dump() {
    let stage2_app = r#"
module stage2.rust_item_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "module demo.app; import demo.math; import demo.text.format as fmt; export import demo.extra.tools; export struct User { id: Int, name: String } enum Result { Ok(Int), Err(String), None } export fn add(a: Int, b: String) -> Bool { return true; } fn ping() { return; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/rust_item_dump.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap Rust item dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  ModuleDecl name=demo.app\n  Import path=demo.math\n  Import path=demo.text.format alias=fmt\n  Import path=demo.extra.tools exported=true\n  Struct name=User exported=true\n    Field name=id type=Int\n    Field name=name type=String\n  Enum name=Result exported=false\n    Variant name=Ok payload=Int\n    Variant name=Err payload=String\n    Variant name=None\n  Function name=add exported=true\n    Param name=a type=Int\n    Param name=b type=String\n    Return type=Bool\n    Return\n      Bool true\n  Function name=ping exported=false\n    Return\n"
    );
}

#[test]
fn stage2_bootstrap_harness_rust_item_dump_ignores_text_noise() {
    let stage2_app = r#"
module stage2.rust_item_noise;
import stage1.frontend;

fn main() -> String {
  let source: String = "module demo.clean; import demo.real as real; export fn main(a: Int) -> Int { let text: String = \"module fake.noise; import fake.path as nope; export struct Hidden { id: Int } enum Noise { Bad(String) } fn nope(x: Int) -> Bool\"; // export fn comment(a: String) -> Bool\n return a; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/rust_item_noise.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap Rust item noise harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  ModuleDecl name=demo.clean\n  Import path=demo.real alias=real\n  Function name=main exported=true\n    Param name=a type=Int\n    Return type=Int\n    Let name=text type=String\n      String \"module fake.noise; import fake.path as nope; export struct Hidden { id: Int } enum Noise { Bad(String) } fn nope(x: Int) -> Bool\"\n    Return\n      Name a\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_body_dump() {
    let stage2_app = r#"
module stage2.rust_body_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Int { let answer: Int = 42; let text: String = \"ready\"; let ok: Bool = true; let alias: Int = answer; let mut count: Int = 1; let inferred = 7; return alias; } fn ping() { return; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/rust_body_dump.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap Rust body dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Int\n    Let name=answer type=Int\n      Int 42\n    Let name=text type=String\n      String \"ready\"\n    Let name=ok type=Bool\n      Bool true\n    Let name=alias type=Int\n      Name answer\n    Let name=count type=Int mutable=true\n      Int 1\n    Let name=inferred\n      Int 7\n    Return\n      Name alias\n  Function name=ping exported=false\n    Return\n"
    );
}

#[test]
fn stage2_bootstrap_harness_rust_body_dump_keeps_v06_boundaries() {
    let stage2_app = r#"
module stage2.rust_body_boundaries;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Int { let text: String = \"let fake: Int = 1; return fake;\"; // return 9;\n if true { let hidden: Int = 1; return hidden; } let sum: Int = 1 + 2; return math.add(sum); }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_body_boundaries.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust body boundary harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Int\n    Let name=text type=String\n      String \"let fake: Int = 1; return fake;\"\n    If\n      Condition\n        Bool true\n      Then\n        Let name=hidden type=Int\n          Int 1\n        Return\n          Name hidden\n    Let name=sum type=Int\n      Binary op=add\n        Int 1\n        Int 2\n    Return\n      Call callee=math.add\n        Name sum\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_assign_dump() {
    let stage2_app = r#"
module stage2.rust_assign_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Int { let mut value: Int = 1; value = value + 1; ready = !flags[0] || check(User { id: value }, [ready]); data = User { id: items[0], tags: [ready] }; return value; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_assign_dump.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust assign dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Int\n    Let name=value type=Int mutable=true\n      Int 1\n    Assign name=value\n      Binary op=add\n        Name value\n        Int 1\n    Assign name=ready\n      Binary op=or\n        Unary op=not\n          Index\n            Base\n              Name flags\n            Index\n              Int 0\n        Call callee=check\n          StructLiteral type=User\n            FieldInit name=id\n              Name value\n          ArrayLiteral\n            Name ready\n    Assign name=data\n      StructLiteral type=User\n        FieldInit name=id\n          Index\n            Base\n              Name items\n            Index\n              Int 0\n        FieldInit name=tags\n          ArrayLiteral\n            Name ready\n    Return\n      Name value\n"
    );
}

#[test]
fn stage2_bootstrap_harness_rust_assign_dump_keeps_v18_boundaries() {
    let stage2_app = r#"
module stage2.rust_assign_boundaries;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Int { let mut value: Int = 1; user.value = 2; items[0] = 3; if true { value = 4; } value = check(outer()); return value; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_assign_boundaries.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust assign boundary harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Int\n    Let name=value type=Int mutable=true\n      Int 1\n    If\n      Condition\n        Bool true\n      Then\n        Assign name=value\n          Int 4\n    Assign name=value\n      Call callee=check\n        Call callee=outer\n    Return\n      Name value\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_control_flow_dump() {
    let stage2_app = r#"
module stage2.rust_control_flow_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Int { let mut value: Int = 0; while value < 3 { if value == 1 { value = value + 1; continue; } else { break; } } return value; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_control_flow_dump.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust control-flow dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Int\n    Let name=value type=Int mutable=true\n      Int 0\n    While\n      Condition\n        Binary op=lt\n          Name value\n          Int 3\n      Body\n        If\n          Condition\n            Binary op=eq\n              Name value\n              Int 1\n          Then\n            Assign name=value\n              Binary op=add\n                Name value\n                Int 1\n            Continue\n          Else\n            Break\n    Return\n      Name value\n"
    );
}

#[test]
fn stage2_bootstrap_harness_rust_control_flow_dump_keeps_boundaries() {
    let stage2_app = r#"
module stage2.rust_control_flow_boundaries;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Int { let text: String = \"if fake { break; } while fake { continue; }\"; // if comment { break; }\n if true == false { return 1; } let mut value: Int = 0; while \"x\" != \"y\" { value = value + 1; } while check(outer()) { value = value + 1; } return value; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_control_flow_boundaries.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust control-flow boundary harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Int\n    Let name=text type=String\n      String \"if fake { break; } while fake { continue; }\"\n    If\n      Condition\n        Binary op=eq\n          Bool true\n          Bool false\n      Then\n        Return\n          Int 1\n    Let name=value type=Int mutable=true\n      Int 0\n    While\n      Condition\n        Binary op=not_eq\n          String \"x\"\n          String \"y\"\n      Body\n        Assign name=value\n          Binary op=add\n            Name value\n            Int 1\n    While\n      Condition\n        Call callee=check\n          Call callee=outer\n      Body\n        Assign name=value\n          Binary op=add\n            Name value\n            Int 1\n    Return\n      Name value\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_match_dump() {
    let stage2_app = r#"
module stage2.rust_match_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Int { let mut value: Int = 0; match value { 0 -> { value = value + 1; }, 1 -> { return value; }, _ -> { return 9; }, } return value; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/rust_match_dump.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap Rust match dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Int\n    Let name=value type=Int mutable=true\n      Int 0\n    Match\n      Value\n        Name value\n      Arm pattern=int:0\n        Assign name=value\n          Binary op=add\n            Name value\n            Int 1\n      Arm pattern=int:1\n        Return\n          Name value\n      Arm pattern=_\n        Return\n          Int 9\n    Return\n      Name value\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_match_pattern_dump() {
    let stage2_app = r#"
module stage2.rust_match_pattern_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "enum Result { Ok(Int), Err(String), None } fn main() -> Int { match Result.Ok(1) { Result.Ok(value) -> { return value; }, Result.Err -> { return 2; }, Result.None -> { return 3; }, true -> { return 4; }, \"done\" -> { return 5; }, name -> { return 6; }, _ -> { return 7; }, } return 0; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_match_pattern_dump.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust match pattern dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Enum name=Result exported=false\n    Variant name=Ok payload=Int\n    Variant name=Err payload=String\n    Variant name=None\n  Function name=main exported=false\n    Return type=Int\n    Match\n      Value\n        Call callee=Result.Ok\n          Int 1\n      Arm pattern=variant:Result.Ok(value)\n        Return\n          Name value\n      Arm pattern=variant:Result.Err\n        Return\n          Int 2\n      Arm pattern=variant:Result.None\n        Return\n          Int 3\n      Arm pattern=bool:true\n        Return\n          Int 4\n      Arm pattern=string:\"done\"\n        Return\n          Int 5\n      Arm pattern=name:name\n        Return\n          Int 6\n      Arm pattern=_\n        Return\n          Int 7\n    Return\n      Int 0\n"
    );
}

#[test]
fn stage2_bootstrap_harness_rust_match_dump_keeps_boundaries() {
    let stage2_app = r#"
module stage2.rust_match_boundaries;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Int { let text: String = \"match fake { _ -> { return 1; } }\"; // match comment { 0 -> { return 2; } }\n match value { 0 -> { if true == false { return 1; } }, _ -> { return 0; } } match \"x\" != \"y\" { _ -> { return 8; } } match check(outer()) { _ -> { return 9; } } return 3; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_match_boundaries.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust match boundary harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Int\n    Let name=text type=String\n      String \"match fake { _ -> { return 1; } }\"\n    Match\n      Value\n        Name value\n      Arm pattern=int:0\n        If\n          Condition\n            Binary op=eq\n              Bool true\n              Bool false\n          Then\n            Return\n              Int 1\n      Arm pattern=_\n        Return\n          Int 0\n    Match\n      Value\n        Binary op=not_eq\n          String \"x\"\n          String \"y\"\n      Arm pattern=_\n        Return\n          Int 8\n    Match\n      Value\n        Call callee=check\n          Call callee=outer\n      Arm pattern=_\n        Return\n          Int 9\n    Return\n      Int 3\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_expr_stmt_dump() {
    let stage2_app = r#"
module stage2.rust_expr_stmt_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Int { ping(); check(ready, value + 1 < limit); value + 1; !done || fallback; user.active; items[0]; [1, 2, value]; User { id: value, active: true }; return 0; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_expr_stmt_dump.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust expr stmt dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Int\n    ExprStmt\n      Call callee=ping\n    ExprStmt\n      Call callee=check\n        Name ready\n        Binary op=lt\n          Binary op=add\n            Name value\n            Int 1\n          Name limit\n    ExprStmt\n      Binary op=add\n        Name value\n        Int 1\n    ExprStmt\n      Binary op=or\n        Unary op=not\n          Name done\n        Name fallback\n    ExprStmt\n      FieldAccess field=active\n        Name user\n    ExprStmt\n      Index\n        Base\n          Name items\n        Index\n          Int 0\n    ExprStmt\n      ArrayLiteral\n        Int 1\n        Int 2\n        Name value\n    ExprStmt\n      StructLiteral type=User\n        FieldInit name=id\n          Name value\n        FieldInit name=active\n          Bool true\n    Return\n      Int 0\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_nested_expr_stmt_dump() {
    let stage2_app = r#"
module stage2.rust_nested_expr_stmt_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Int { while ready { tick(); if done { finish(); } else { retry(); } } match value { 0 -> { handle_zero(); }, _ -> { handle_other(); }, } return 0; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_nested_expr_stmt_dump.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust nested expr stmt dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Int\n    While\n      Condition\n        Name ready\n      Body\n        ExprStmt\n          Call callee=tick\n        If\n          Condition\n            Name done\n          Then\n            ExprStmt\n              Call callee=finish\n          Else\n            ExprStmt\n              Call callee=retry\n    Match\n      Value\n        Name value\n      Arm pattern=int:0\n        ExprStmt\n          Call callee=handle_zero\n      Arm pattern=_\n        ExprStmt\n          Call callee=handle_other\n    Return\n      Int 0\n"
    );
}

#[test]
fn stage2_bootstrap_harness_rust_expr_stmt_dump_keeps_boundaries() {
    let stage2_app = r#"
module stage2.rust_expr_stmt_boundaries;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Int { let text: String = \"ping(); check(value);\"; // ping();\n math.check(value); make()[0]; [ready][0]; User { id: ready() }; return 0; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_expr_stmt_boundaries.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust expr stmt boundary harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Int\n    Let name=text type=String\n      String \"ping(); check(value);\"\n    ExprStmt\n      Call callee=math.check\n        Name value\n    ExprStmt\n      Index\n        Base\n          Call callee=make\n        Index\n          Int 0\n    ExprStmt\n      Index\n        Base\n          ArrayLiteral\n            Name ready\n        Index\n          Int 0\n    ExprStmt\n      StructLiteral type=User\n        FieldInit name=id\n          Call callee=ready\n    Return\n      Int 0\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_binary_dump() {
    let stage2_app = r#"
module stage2.rust_binary_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Int { let base: Int = 1 + 2 * 3; let sum: Int = 1 + 2 + 3; return base * 4 * 5; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_binary_dump.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust binary dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Int\n    Let name=base type=Int\n      Binary op=add\n        Int 1\n        Binary op=mul\n          Int 2\n          Int 3\n    Let name=sum type=Int\n      Binary op=add\n        Binary op=add\n          Int 1\n          Int 2\n        Int 3\n    Return\n      Binary op=mul\n        Binary op=mul\n          Name base\n          Int 4\n        Int 5\n"
    );
}

#[test]
fn stage2_bootstrap_harness_rust_binary_dump_keeps_v07_boundaries() {
    let stage2_app = r#"
module stage2.rust_binary_boundaries;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Int { let text: String = \"a\" + \"b\"; return [text][0]; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_binary_boundaries.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust binary boundary harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Int\n    Let name=text type=String\n    Return\n      Index\n        Base\n          ArrayLiteral\n            Name text\n        Index\n          Int 0\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_paren_binary_dump() {
    let stage2_app = r#"
module stage2.rust_paren_binary_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Int { let grouped: Int = (1 + 2) * 3; let right: Int = 1 * (2 + 3); return (grouped + right) * 4; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_paren_binary_dump.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust paren binary dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Int\n    Let name=grouped type=Int\n      Binary op=mul\n        Binary op=add\n          Int 1\n          Int 2\n        Int 3\n    Let name=right type=Int\n      Binary op=mul\n        Int 1\n        Binary op=add\n          Int 2\n          Int 3\n    Return\n      Binary op=mul\n        Binary op=add\n          Name grouped\n          Name right\n        Int 4\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_arithmetic_dump() {
    let stage2_app = r#"
module stage2.rust_arithmetic_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Int { let value: Int = 10 - 2 * 3 + 8 / 4; return value * 6 / 2; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_arithmetic_dump.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust arithmetic dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Int\n    Let name=value type=Int\n      Binary op=add\n        Binary op=sub\n          Int 10\n          Binary op=mul\n            Int 2\n            Int 3\n        Binary op=div\n          Int 8\n          Int 4\n    Return\n      Binary op=div\n        Binary op=mul\n          Name value\n          Int 6\n        Int 2\n"
    );
}

#[test]
fn stage2_bootstrap_harness_rust_arithmetic_dump_keeps_v09_boundaries() {
    let stage2_app = r#"
module stage2.rust_arithmetic_boundaries;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Int { let neg: Int = -1; let call: Int = add(-1); let named: Int = -value; let grouped: Int = -(value + 1); let right: Int = value - -1; let product: Int = value * -1; let mixed: Int = true - 1; return -value; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_arithmetic_boundaries.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust arithmetic boundary harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Int\n    Let name=neg type=Int\n      Unary op=neg\n        Int 1\n    Let name=call type=Int\n      Call callee=add\n        Unary op=neg\n          Int 1\n    Let name=named type=Int\n      Unary op=neg\n        Name value\n    Let name=grouped type=Int\n      Unary op=neg\n        Binary op=add\n          Name value\n          Int 1\n    Let name=right type=Int\n      Binary op=sub\n        Name value\n        Unary op=neg\n          Int 1\n    Let name=product type=Int\n      Binary op=mul\n        Name value\n        Unary op=neg\n          Int 1\n    Let name=mixed type=Int\n    Return\n      Unary op=neg\n        Name value\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_comparison_dump() {
    let stage2_app = r#"
module stage2.rust_comparison_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Bool { let lt: Bool = 1 + 2 < 4 * 2; let eq: Bool = value == 10; let ne: Bool = value != 0; let lte: Bool = value <= 10; let gt: Bool = value > 1; return value + 1 >= 3; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_comparison_dump.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust comparison dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Bool\n    Let name=lt type=Bool\n      Binary op=lt\n        Binary op=add\n          Int 1\n          Int 2\n        Binary op=mul\n          Int 4\n          Int 2\n    Let name=eq type=Bool\n      Binary op=eq\n        Name value\n        Int 10\n    Let name=ne type=Bool\n      Binary op=not_eq\n        Name value\n        Int 0\n    Let name=lte type=Bool\n      Binary op=lte\n        Name value\n        Int 10\n    Let name=gt type=Bool\n      Binary op=gt\n        Name value\n        Int 1\n    Return\n      Binary op=gte\n        Binary op=add\n          Name value\n          Int 1\n        Int 3\n"
    );
}

#[test]
fn stage2_bootstrap_harness_rust_comparison_dump_keeps_v10_boundaries() {
    let stage2_app = r#"
module stage2.rust_comparison_boundaries;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Bool { let bools: Bool = true == false; let strings: Bool = \"a\" != \"b\"; let grouped: Bool = (true) == (false); let bool_order: Bool = true < false; let string_order: Bool = \"a\" < \"b\"; let chain: Bool = 1 < 2 < 3; let path: Bool = math.check(ready); return math.check(path); }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_comparison_boundaries.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust comparison boundary harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Bool\n    Let name=bools type=Bool\n      Binary op=eq\n        Bool true\n        Bool false\n    Let name=strings type=Bool\n      Binary op=not_eq\n        String \"a\"\n        String \"b\"\n    Let name=grouped type=Bool\n      Binary op=eq\n        Bool true\n        Bool false\n    Let name=bool_order type=Bool\n    Let name=string_order type=Bool\n    Let name=chain type=Bool\n    Let name=path type=Bool\n      Call callee=math.check\n        Name ready\n    Return\n      Call callee=math.check\n        Name path\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_logic_dump() {
    let stage2_app = r#"
module stage2.rust_logic_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Bool { let ok: Bool = a < b && c >= d || ready; let grouped: Bool = a && (b || c); return value + 1 < limit && done; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/rust_logic_dump.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap Rust logic dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Bool\n    Let name=ok type=Bool\n      Binary op=or\n        Binary op=and\n          Binary op=lt\n            Name a\n            Name b\n          Binary op=gte\n            Name c\n            Name d\n        Name ready\n    Let name=grouped type=Bool\n      Binary op=and\n        Name a\n        Binary op=or\n          Name b\n          Name c\n    Return\n      Binary op=and\n        Binary op=lt\n          Binary op=add\n            Name value\n            Int 1\n          Name limit\n        Name done\n"
    );
}

#[test]
fn stage2_bootstrap_harness_rust_logic_dump_keeps_v11_boundaries() {
    let stage2_app = r#"
module stage2.rust_logic_boundaries;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Bool { let nested: Bool = outer(check(a && b)); let mixed: Bool = true == false; let text: Bool = \"x\" == \"x\"; let ints: Bool = 1 && 2; let strings: Bool = \"x\" || ready; return outer(check(a && b)); }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_logic_boundaries.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust logic boundary harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Bool\n    Let name=nested type=Bool\n      Call callee=outer\n        Call callee=check\n          Binary op=and\n            Name a\n            Name b\n    Let name=mixed type=Bool\n      Binary op=eq\n        Bool true\n        Bool false\n    Let name=text type=Bool\n      Binary op=eq\n        String \"x\"\n        String \"x\"\n    Let name=ints type=Bool\n    Let name=strings type=Bool\n    Return\n      Call callee=outer\n        Call callee=check\n          Binary op=and\n            Name a\n            Name b\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_unary_dump() {
    let stage2_app = r#"
module stage2.rust_unary_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Bool { let notted: Bool = !ready; let grouped: Bool = !(a && b || c); let compared: Bool = !(value + 1 < limit); return !!done || !failed && ready; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/rust_unary_dump.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap Rust unary dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Bool\n    Let name=notted type=Bool\n      Unary op=not\n        Name ready\n    Let name=grouped type=Bool\n      Unary op=not\n        Binary op=or\n          Binary op=and\n            Name a\n            Name b\n          Name c\n    Let name=compared type=Bool\n      Unary op=not\n        Binary op=lt\n          Binary op=add\n            Name value\n            Int 1\n          Name limit\n    Return\n      Binary op=or\n        Unary op=not\n          Unary op=not\n            Name done\n        Binary op=and\n          Unary op=not\n            Name failed\n          Name ready\n"
    );
}

#[test]
fn stage2_bootstrap_harness_rust_unary_dump_keeps_v12_boundaries() {
    let stage2_app = r#"
module stage2.rust_unary_boundaries;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Bool { let neg: Int = -1; let named: Int = -value; let grouped: Int = -(value + 1); let str_not: Bool = !\"x\"; let int_not: Bool = !1; return -value; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_unary_boundaries.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust unary boundary harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Bool\n    Let name=neg type=Int\n      Unary op=neg\n        Int 1\n    Let name=named type=Int\n      Unary op=neg\n        Name value\n    Let name=grouped type=Int\n      Unary op=neg\n        Binary op=add\n          Name value\n          Int 1\n    Let name=str_not type=Bool\n    Let name=int_not type=Bool\n    Return\n      Unary op=neg\n        Name value\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_call_dump() {
    let stage2_app = r#"
module stage2.rust_call_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Bool { let pinged: Bool = ping(); let checked: Bool = check(ready, value + 1 < limit); let mixed: Bool = !check(a && b) || done; return add(1, value) * 2 < limit && ok(); }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/rust_call_dump.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap Rust call dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Bool\n    Let name=pinged type=Bool\n      Call callee=ping\n    Let name=checked type=Bool\n      Call callee=check\n        Name ready\n        Binary op=lt\n          Binary op=add\n            Name value\n            Int 1\n          Name limit\n    Let name=mixed type=Bool\n      Binary op=or\n        Unary op=not\n          Call callee=check\n            Binary op=and\n              Name a\n              Name b\n        Name done\n    Return\n      Binary op=and\n        Binary op=lt\n          Binary op=mul\n            Call callee=add\n              Int 1\n              Name value\n            Int 2\n          Name limit\n        Call callee=ok\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_qualified_call_dump() {
    let stage2_app = r#"
module stage2.rust_qualified_call_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Bool { let printed: Bool = std.io.print(value); let checked: Bool = math.logic.check(ready, value + 1 < limit); Result.Ok(1); return std.core.ok() && Result.Ok(1); }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_qualified_call_dump.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust qualified call dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Bool\n    Let name=printed type=Bool\n      Call callee=std.io.print\n        Name value\n    Let name=checked type=Bool\n      Call callee=math.logic.check\n        Name ready\n        Binary op=lt\n          Binary op=add\n            Name value\n            Int 1\n          Name limit\n    ExprStmt\n      Call callee=Result.Ok\n        Int 1\n    Return\n      Binary op=and\n        Call callee=std.core.ok\n        Call callee=Result.Ok\n          Int 1\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_nested_call_arg_dump() {
    let stage2_app = r#"
module stage2.rust_nested_call_arg_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Bool { let nested: Bool = outer(inner()); let checked: Bool = check(Result.Ok(1), std.core.ok()); let mixed: Bool = wrap(value + inner(1) < limit, !ready()); return outer(inner(value), check(Result.Ok(1))); }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_nested_call_arg_dump.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust nested call arg dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Bool\n    Let name=nested type=Bool\n      Call callee=outer\n        Call callee=inner\n    Let name=checked type=Bool\n      Call callee=check\n        Call callee=Result.Ok\n          Int 1\n        Call callee=std.core.ok\n    Let name=mixed type=Bool\n      Call callee=wrap\n        Binary op=lt\n          Binary op=add\n            Name value\n            Call callee=inner\n              Int 1\n          Name limit\n        Unary op=not\n          Call callee=ready\n    Return\n      Call callee=outer\n        Call callee=inner\n          Name value\n        Call callee=check\n          Call callee=Result.Ok\n            Int 1\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_call_container_dump() {
    let stage2_app = r#"
module stage2.rust_call_container_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> User { let values: IntArray = [1, next(), Result.Ok(2), User { id: 3 }, true == false]; let item: Int = items[index_of(value)]; let user: User = User { id: next(), active: check(Result.Ok(1)), child: User { id: 1, active: true }, ready: \"x\" != \"y\" }; let passed: Bool = check(User { id: 1, active: true }, value, true == false); return User { id: items[ready_index()], child: User { id: make_id() }, active: \"x\" == \"x\" }; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_call_container_dump.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust call container dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=User\n    Let name=values type=IntArray\n      ArrayLiteral\n        Int 1\n        Call callee=next\n        Call callee=Result.Ok\n          Int 2\n        StructLiteral type=User\n          FieldInit name=id\n            Int 3\n        Binary op=eq\n          Bool true\n          Bool false\n    Let name=item type=Int\n      Index\n        Base\n          Name items\n        Index\n          Call callee=index_of\n            Name value\n    Let name=user type=User\n      StructLiteral type=User\n        FieldInit name=id\n          Call callee=next\n        FieldInit name=active\n          Call callee=check\n            Call callee=Result.Ok\n              Int 1\n        FieldInit name=child\n          StructLiteral type=User\n            FieldInit name=id\n              Int 1\n            FieldInit name=active\n              Bool true\n        FieldInit name=ready\n          Binary op=not_eq\n            String \"x\"\n            String \"y\"\n    Let name=passed type=Bool\n      Call callee=check\n        StructLiteral type=User\n          FieldInit name=id\n            Int 1\n          FieldInit name=active\n            Bool true\n        Name value\n        Binary op=eq\n          Bool true\n          Bool false\n    Return\n      StructLiteral type=User\n        FieldInit name=id\n          Index\n            Base\n              Name items\n            Index\n              Call callee=ready_index\n        FieldInit name=child\n          StructLiteral type=User\n            FieldInit name=id\n              Call callee=make_id\n        FieldInit name=active\n          Binary op=eq\n            String \"x\"\n            String \"x\"\n"
    );
}

#[test]
fn stage2_bootstrap_harness_rust_call_dump_keeps_v13_boundaries() {
    let stage2_app = r#"
module stage2.rust_call_boundaries;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Bool { let path: Bool = math.check(ready); let enum_like: Bool = Result.Ok(1); let nested: Bool = outer(inner()); let payload: Bool = check(Result.Ok(1)); let method: Bool = user.check(); let call_index: Bool = make()[0]; let suffix: Bool = make().ready; let struct_suffix_arg: Bool = check(User { active: true }.active); let neg_arg: Bool = check(-1); return std.core.ok(); }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_call_boundaries.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust call boundary harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Bool\n    Let name=path type=Bool\n      Call callee=math.check\n        Name ready\n    Let name=enum_like type=Bool\n      Call callee=Result.Ok\n        Int 1\n    Let name=nested type=Bool\n      Call callee=outer\n        Call callee=inner\n    Let name=payload type=Bool\n      Call callee=check\n        Call callee=Result.Ok\n          Int 1\n    Let name=method type=Bool\n      Call callee=user.check\n    Let name=call_index type=Bool\n      Index\n        Base\n          Call callee=make\n        Index\n          Int 0\n    Let name=suffix type=Bool\n      FieldAccess field=ready\n        Call callee=make\n    Let name=struct_suffix_arg type=Bool\n      Call callee=check\n        FieldAccess field=active\n          StructLiteral type=User\n            FieldInit name=active\n              Bool true\n    Let name=neg_arg type=Bool\n      Call callee=check\n        Unary op=neg\n          Int 1\n    Return\n      Call callee=std.core.ok\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_field_dump() {
    let stage2_app = r#"
module stage2.rust_field_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Bool { let active: Bool = user.active; let score: Int = user.profile.score + 1; let mixed: Bool = !user.enabled || ready; return config.limit > value && session.ready; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/rust_field_dump.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap Rust field dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Bool\n    Let name=active type=Bool\n      FieldAccess field=active\n        Name user\n    Let name=score type=Int\n      Binary op=add\n        FieldAccess field=score\n          FieldAccess field=profile\n            Name user\n        Int 1\n    Let name=mixed type=Bool\n      Binary op=or\n        Unary op=not\n          FieldAccess field=enabled\n            Name user\n        Name ready\n    Return\n      Binary op=and\n        Binary op=gt\n          FieldAccess field=limit\n            Name config\n          Name value\n        FieldAccess field=ready\n          Name session\n"
    );
}

#[test]
fn stage2_bootstrap_harness_rust_field_dump_keeps_v14_boundaries() {
    let stage2_app = r#"
module stage2.rust_field_boundaries;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Bool { let dotted_call: Bool = math.check(ready); let method: Bool = user.check(); let call_suffix: Bool = make().ready; let indexed: Bool = items[0].ready; return math.check(ready); }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_field_boundaries.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust field boundary harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Bool\n    Let name=dotted_call type=Bool\n      Call callee=math.check\n        Name ready\n    Let name=method type=Bool\n      Call callee=user.check\n    Let name=call_suffix type=Bool\n      FieldAccess field=ready\n        Call callee=make\n    Let name=indexed type=Bool\n      FieldAccess field=ready\n        Index\n          Base\n            Name items\n          Index\n            Int 0\n    Return\n      Call callee=math.check\n        Name ready\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_index_dump() {
    let stage2_app = r#"
module stage2.rust_index_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Bool { let first: Int = items[0]; let dynamic: Int = items[index + 1]; let chained: Int = matrix[row][col]; let mixed: Bool = !flags[0] || check(items[index], values[1] < limit); return items[0] + 1 < limit && flags[index]; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/rust_index_dump.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap Rust index dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Bool\n    Let name=first type=Int\n      Index\n        Base\n          Name items\n        Index\n          Int 0\n    Let name=dynamic type=Int\n      Index\n        Base\n          Name items\n        Index\n          Binary op=add\n            Name index\n            Int 1\n    Let name=chained type=Int\n      Index\n        Base\n          Index\n            Base\n              Name matrix\n            Index\n              Name row\n        Index\n          Name col\n    Let name=mixed type=Bool\n      Binary op=or\n        Unary op=not\n          Index\n            Base\n              Name flags\n            Index\n              Int 0\n        Call callee=check\n          Index\n            Base\n              Name items\n            Index\n              Name index\n          Binary op=lt\n            Index\n              Base\n                Name values\n              Index\n                Int 1\n            Name limit\n    Return\n      Binary op=and\n        Binary op=lt\n          Binary op=add\n            Index\n              Base\n                Name items\n              Index\n                Int 0\n            Int 1\n          Name limit\n        Index\n          Base\n            Name flags\n          Index\n            Name index\n"
    );
}

#[test]
fn stage2_bootstrap_harness_rust_index_dump_keeps_v15_boundaries() {
    let stage2_app = r#"
module stage2.rust_index_boundaries;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Bool { let array: Bool = [ready][0]; let call_suffix: Bool = make()[0]; let field_index: Bool = user.items[0]; let index_field: Bool = items[0].ready; let struct_index: Bool = User { active: true }[0]; let nested_call: Bool = items[check()]; let neg_index: Bool = items[-1]; return check(items[ready()]); }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_index_boundaries.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust index boundary harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Bool\n    Let name=array type=Bool\n      Index\n        Base\n          ArrayLiteral\n            Name ready\n        Index\n          Int 0\n    Let name=call_suffix type=Bool\n      Index\n        Base\n          Call callee=make\n        Index\n          Int 0\n    Let name=field_index type=Bool\n      Index\n        Base\n          FieldAccess field=items\n            Name user\n        Index\n          Int 0\n    Let name=index_field type=Bool\n      FieldAccess field=ready\n        Index\n          Base\n            Name items\n          Index\n            Int 0\n    Let name=struct_index type=Bool\n      Index\n        Base\n          StructLiteral type=User\n            FieldInit name=active\n              Bool true\n        Index\n          Int 0\n    Let name=nested_call type=Bool\n      Index\n        Base\n          Name items\n        Index\n          Call callee=check\n    Let name=neg_index type=Bool\n      Index\n        Base\n          Name items\n        Index\n          Unary op=neg\n            Int 1\n    Return\n      Call callee=check\n        Index\n          Base\n            Name items\n          Index\n            Call callee=ready\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_array_dump() {
    let stage2_app = r#"
module stage2.rust_array_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> IntArray { let empty: IntArray = []; let values: IntArray = [1, value + 1, items[0],]; let flags: BoolArray = [ready, !done || fallback]; let passed: Bool = check([ready, flags[0]], [value < limit]); return [1, 2, 3]; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/rust_array_dump.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap Rust array dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=IntArray\n    Let name=empty type=IntArray\n      ArrayLiteral\n    Let name=values type=IntArray\n      ArrayLiteral\n        Int 1\n        Binary op=add\n          Name value\n          Int 1\n        Index\n          Base\n            Name items\n          Index\n            Int 0\n    Let name=flags type=BoolArray\n      ArrayLiteral\n        Name ready\n        Binary op=or\n          Unary op=not\n            Name done\n          Name fallback\n    Let name=passed type=Bool\n      Call callee=check\n        ArrayLiteral\n          Name ready\n          Index\n            Base\n              Name flags\n            Index\n              Int 0\n        ArrayLiteral\n          Binary op=lt\n            Name value\n            Name limit\n    Return\n      ArrayLiteral\n        Int 1\n        Int 2\n        Int 3\n"
    );
}

#[test]
fn stage2_bootstrap_harness_rust_array_dump_keeps_v16_boundaries() {
    let stage2_app = r#"
module stage2.rust_array_boundaries;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Bool { let suffix_index: Bool = [ready][0]; let suffix_field: Bool = [ready].active; let struct_item: BoolArray = [User { active: true }]; let nested_call: BoolArray = [check()]; let neg_item: IntArray = [-1]; return check([ready()]); }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_array_boundaries.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust array boundary harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Bool\n    Let name=suffix_index type=Bool\n      Index\n        Base\n          ArrayLiteral\n            Name ready\n        Index\n          Int 0\n    Let name=suffix_field type=Bool\n      FieldAccess field=active\n        ArrayLiteral\n          Name ready\n    Let name=struct_item type=BoolArray\n      ArrayLiteral\n        StructLiteral type=User\n          FieldInit name=active\n            Bool true\n    Let name=nested_call type=BoolArray\n      ArrayLiteral\n        Call callee=check\n    Let name=neg_item type=IntArray\n      ArrayLiteral\n        Unary op=neg\n          Int 1\n    Return\n      Call callee=check\n        ArrayLiteral\n          Call callee=ready\n"
    );
}

#[test]
fn stage2_bootstrap_harness_reads_stage1_rust_struct_expr_dump() {
    let stage2_app = r#"
module stage2.rust_struct_expr_dump;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> User { let empty: User = User {}; let basic: User = User { id: 1, active: true, }; let computed: User = User { id: value + 1, ready: !failed, item: items[0], tags: [ready, done] }; let passed: Bool = check(User { id: 1 }, user.active); return User { id: value }; }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_struct_expr_dump.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust struct expr dump harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=User\n    Let name=empty type=User\n      StructLiteral type=User\n    Let name=basic type=User\n      StructLiteral type=User\n        FieldInit name=id\n          Int 1\n        FieldInit name=active\n          Bool true\n    Let name=computed type=User\n      StructLiteral type=User\n        FieldInit name=id\n          Binary op=add\n            Name value\n            Int 1\n        FieldInit name=ready\n          Unary op=not\n            Name failed\n        FieldInit name=item\n          Index\n            Base\n              Name items\n            Index\n              Int 0\n        FieldInit name=tags\n          ArrayLiteral\n            Name ready\n            Name done\n    Let name=passed type=Bool\n      Call callee=check\n        StructLiteral type=User\n          FieldInit name=id\n            Int 1\n        FieldAccess field=active\n          Name user\n    Return\n      StructLiteral type=User\n        FieldInit name=id\n          Name value\n"
    );
}

#[test]
fn stage2_bootstrap_harness_rust_struct_expr_dump_keeps_v17_boundaries() {
    let stage2_app = r#"
module stage2.rust_struct_expr_boundaries;
import stage1.frontend;

fn main() -> String {
  let source: String = "fn main() -> Bool { let suffix_field: Bool = User { active: true }.active; let suffix_index: Bool = User { active: true }[0]; let nested_struct: User = User { child: User { id: 1 } }; let struct_array: User = User { items: [User { id: 1 }] }; let call_field: User = User { id: check() }; let neg_field: User = User { id: -1 }; return check(User { id: ready() }); }";
  return stage1.frontend.ast_dump_rust_item_dump(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file(
            "testdata/stage2_bootstrap/rust_struct_expr_boundaries.zeta",
            stage2_app,
        ),
    ])
    .expect("Stage2 bootstrap Rust struct expr boundary harness should run");

    assert_eq!(
        value.to_string(),
        "Module\n  Function name=main exported=false\n    Return type=Bool\n    Let name=suffix_field type=Bool\n      FieldAccess field=active\n        StructLiteral type=User\n          FieldInit name=active\n            Bool true\n    Let name=suffix_index type=Bool\n      Index\n        Base\n          StructLiteral type=User\n            FieldInit name=active\n              Bool true\n        Index\n          Int 0\n    Let name=nested_struct type=User\n      StructLiteral type=User\n        FieldInit name=child\n          StructLiteral type=User\n            FieldInit name=id\n              Int 1\n    Let name=struct_array type=User\n      StructLiteral type=User\n        FieldInit name=items\n          ArrayLiteral\n            StructLiteral type=User\n              FieldInit name=id\n                Int 1\n    Let name=call_field type=User\n      StructLiteral type=User\n        FieldInit name=id\n          Call callee=check\n    Let name=neg_field type=User\n      StructLiteral type=User\n        FieldInit name=id\n          Unary op=neg\n            Int 1\n    Return\n      Call callee=check\n        StructLiteral type=User\n          FieldInit name=id\n            Call callee=ready\n"
    );
}

#[test]
fn stage2_bootstrap_harness_rejects_single_logical_symbols_in_stage1_summary() {
    let stage2_app = r#"
module stage2.logical_symbols;
import std.core;
import stage1.frontend;

fn main() -> String {
  let source: String = "& | && ||";
  let symbols: String = stage1.frontend.ast_dump_symbol_summary(source);
  let classes: String = stage1.frontend.ast_dump_token_class_summary(source);
  return string_concat(string_concat(symbols, "|"), classes);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/logical_symbols.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap logical symbol summary harness should run");

    assert_eq!(
        value.to_string(),
        "lparen=0;rparen=0;lbrace=0;rbrace=0;lbracket=0;rbracket=0;colon=0;semicolon=0;comma=0;dot=0;arrow=0;eq=0;eqeq=0;bang=0;bangeq=0;andand=1;oror=1;lt=0;lte=0;gt=0;gte=0;plus=0;minus=0;star=0;slash=0|ident=0;int=0;string=0;bool=0;unknown=2;eof=1"
    );
}

#[test]
fn stage2_bootstrap_harness_keeps_escaped_quote_noise_inside_stage1_string() {
    let stage2_app = r#"
module stage2.escaped_string;
import stage1.frontend;

fn main() -> String {
  let source: String = "\"alpha \\\" // fn { import demo.noise; }\" fn";
  return stage1.frontend.ast_dump_keyword_summary(source);
}
"#;
    let value = zeta::module_graph::run_sources(&[
        source_file(
            "testdata/stage1_frontend/frontend.zeta",
            include_str!("../testdata/stage1_frontend/frontend.zeta"),
        ),
        source_file("testdata/stage2_bootstrap/escaped_string.zeta", stage2_app),
    ])
    .expect("Stage2 bootstrap escaped string summary harness should run");

    assert_eq!(
        value.to_string(),
        "module=0;import=0;as=0;export=0;fn=1;let=0;mut=0;return=0;break=0;continue=0;if=0;else=0;while=0;match=0;struct=0;enum=0"
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
