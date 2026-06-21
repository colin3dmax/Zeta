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
fn dumps_bitwise_expressions() {
    let dump = zeta::dump_ast(
        r#"
fn main() -> Int {
  return a & b | c ^ ~d;
}
"#,
    )
    .expect("source should parse");

    assert!(dump.contains("Binary op=bit_and"));
    assert!(dump.contains("Binary op=bit_or"));
    assert!(dump.contains("Binary op=bit_xor"));
    assert!(dump.contains("Unary op=bit_not"));
}

#[test]
fn dumps_bitwise_precedence_left_assoc() {
    // `a & b | c` 同级左结合 => (a & b) | c。
    let dump = zeta::dump_ast(
        r#"
fn main() -> Int {
  return a & b | c;
}
"#,
    )
    .expect("source should parse");

    assert_eq!(
        dump,
        "Module\n  Function name=main exported=false\n    Return type=Int\n    Return\n      Binary op=bit_or\n        Binary op=bit_and\n          Name a\n          Name b\n        Name c\n"
    );
}

#[test]
fn dumps_else_if_chain_as_nested_if() {
    let dump = zeta::dump_ast(
        r#"
fn main() -> Int {
  if a < 1 { return 1; } else if a < 2 { return 2; } else { return 3; }
}
"#,
    )
    .expect("source should parse");

    // `else if` desugars to a nested If under the Else branch.
    assert!(dump.contains("Else\n"));
    assert_eq!(dump.matches("If\n").count(), 2);
}

#[test]
fn dumps_for_in_loop() {
    let dump = zeta::dump_ast(
        r#"
fn main() -> Int {
  let mut sum: Int = 0;
  for n in [10, 20, 30] {
    sum = sum + n;
  }
  return sum;
}
"#,
    )
    .expect("source should parse");

    assert!(dump.contains("For binding=n\n"));
    assert!(dump.contains("  Iterable\n"));
    assert!(dump.contains("  Body\n"));
    // iterable array literal then body assignment appear under the for node.
    assert!(dump.contains("ArrayLiteral\n"));
}

#[test]
fn dumps_for_range_loop() {
    let dump = zeta::dump_ast(
        r#"
fn main() -> Int {
  let mut sum: Int = 0;
  for i in 0..n + 1 {
    sum = sum + i;
  }
  return sum;
}
"#,
    )
    .expect("source should parse");

    assert!(dump.contains("For binding=i\n"));
    assert!(dump.contains("  Iterable\n"));
    // the iterable is a Range node carrying the start and end expressions.
    assert!(dump.contains("Range\n"));
    assert!(dump.contains("Int 0\n"));
    assert!(dump.contains("Binary op=add\n"));
    assert!(dump.contains("  Body\n"));
}

#[test]
fn dumps_for_c_loop() {
    let dump = zeta::dump_ast(
        r#"
fn main() -> Int {
  let mut sum: Int = 0;
  for (let mut i: Int = 0; i < 5; i += 1) {
    sum = sum + i;
  }
  return sum;
}
"#,
    )
    .expect("source should parse");

    let expected = "\
    ForC\n      \
      Init\n        \
        Let name=i type=Int mutable=true\n          \
          Int 0\n      \
      Condition\n        \
        Binary op=lt\n          \
          Name i\n          \
          Int 5\n      \
      Step\n        \
        Assign name=i\n          \
          Binary op=add\n            \
            Name i\n            \
            Int 1\n      \
      Body\n        \
        Assign name=sum\n";
    assert!(
        dump.contains(expected),
        "ForC dump did not match; got:\n{dump}"
    );
}

#[test]
fn dumps_complex_assignment_targets() {
    let dump = zeta::dump_ast(
        r#"
fn main() {
  p.x = 1;
  arr[0] = 2;
}
"#,
    )
    .expect("source should parse");

    // 简单变量赋值用 `Assign name=`;字段/下标赋值用 Target/Value 段。
    assert!(dump.contains("Assign\n"));
    assert!(dump.contains("Target\n"));
    assert!(dump.contains("FieldAccess field=x"));
    assert!(dump.contains("Value\n"));
}

#[test]
fn dumps_modulo_with_precedence() {
    let dump = zeta::dump_ast(
        r#"
fn main() -> Int {
  return a % b + c;
}
"#,
    )
    .expect("source should parse");

    // `%` binds tighter than `+`: add(mod(a, b), c)
    assert!(dump.contains("Binary op=mod"));
    assert!(dump.contains("Binary op=add"));
}

#[test]
fn dumps_compound_assignment_as_desugared_binary() {
    let dump = zeta::dump_ast(
        r#"
fn main() {
  a += 3;
}
"#,
    )
    .expect("source should parse");

    // `a += 3` desugars to `a = a + 3`: Assign name=a / Binary op=add / Name a / Int 3
    assert!(dump.contains("Assign name=a"));
    assert!(dump.contains("Binary op=add"));
    assert!(dump.contains("Name a"));
    assert!(dump.contains("Int 3"));
}

#[test]
fn dumps_trait_declaration() {
    let dump = zeta::dump_ast(
        r#"
trait Show {
  fn show(self: Self) -> String;
  fn rank(self: Self, base: Int) -> Int;
}
"#,
    )
    .expect("source should parse");

    assert_eq!(
        dump,
        "\
Module
  Trait name=Show exported=false
    Method name=show
      Param name=self type=Self
      Return type=String
    Method name=rank
      Param name=self type=Self
      Param name=base type=Int
      Return type=Int
"
    );
}

#[test]
fn dumps_impl_block_with_method_bodies() {
    let dump = zeta::dump_ast(
        r#"
impl Show for Point {
  fn show(self: Self) -> String {
    return "point";
  }
}
"#,
    )
    .expect("source should parse");

    assert_eq!(
        dump,
        "\
Module
  Impl trait=Show target=Point exported=false
    Method name=show
      Param name=self type=Self
      Return type=String
      Return
        String \"point\"
"
    );
}

#[test]
fn dumps_generic_trait_and_impl() {
    // `trait`/`impl` are contextual identifiers, so generic args on the trait
    // reference (`Container<T>`) and target (`Box<T>`) round-trip in the dump.
    let dump = zeta::dump_ast(
        r#"
export trait Container<T> {
  fn get(self: Self, i: Int) -> T;
}

impl<T> Container<T> for Box<T> {
  fn get(self: Self, i: Int) -> T {
    return self.value;
  }
}
"#,
    )
    .expect("source should parse");

    assert!(dump.contains("Trait name=Container exported=true"));
    assert!(dump.contains("Impl trait=Container<T> target=Box<T> exported=false"));
}

#[test]
fn trait_and_impl_are_contextual_not_reserved() {
    // Because `trait`/`impl` stay plain identifiers, they remain usable as
    // ordinary names elsewhere — guarding self-hosting token parity.
    let dump = zeta::dump_ast(
        r#"
fn main() -> Int {
  let trait: Int = 1;
  let impl: Int = 2;
  return trait + impl;
}
"#,
    )
    .expect("source should parse");

    assert!(dump.contains("Let name=trait type=Int"));
    assert!(dump.contains("Let name=impl type=Int"));
}

#[test]
fn dumps_generic_function_with_trait_bound() {
    let dump = zeta::dump_ast(
        r#"
fn describe<T: Show>(item: T) -> Int {
  return 0;
}
"#,
    )
    .expect("source should parse");

    assert!(dump.contains("Function name=describe"));
    assert!(dump.contains("Bound param=T trait=Show"));
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
fn repl_accepts_float_literal() {
    // Float literals are now supported (Phase 1); the old rejection is gone.
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
            .write_all(b"2.5\n:quit\n")
            .expect("repl input should write");
    }

    let output = child.wait_with_output().expect("repl should exit");
    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf-8");
    assert!(!stderr.contains("LEX_FLOAT_UNSUPPORTED"));
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
