// Native backend — struct subset differential tests (cargo feature `llvm`).
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_struct
//
// Structs are value types: literals, field read/write, struct locals/params/
// returns, and nesting. For every program the JIT-compiled native result must
// equal the Stage0 interpreter's (the differential oracle).
#![cfg(feature = "llvm")]

use zeta::ast::Item;
use zeta::runtime::Value;

/// Run `fn main() -> Int { ... }` through both the interpreter and the native
/// JIT (passing the program's struct decls) and assert they agree.
fn check(source: &str) -> i64 {
    let module = zeta::parse_source(source).expect("should parse");
    let structs: Vec<zeta::ast::StructDecl> = module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(decl) => Some(decl.clone()),
            _ => None,
        })
        .collect();

    let program = zeta::lower_source(source).expect("should lower");
    let oracle = match zeta::runtime::run_mir(&program).expect("interpreter should run") {
        Value::Int(n) => n,
        other => panic!("expected Int, got {other:?}"),
    };
    let native = zeta::codegen::jit_run_i64(&program, &structs, "main").expect("native JIT");

    assert_eq!(
        native, oracle,
        "native/interpreter divergence\n--- source ---\n{source}\n--- native={native} oracle={oracle} ---"
    );
    oracle
}

#[test]
fn struct_literal_and_field_read() {
    let src = "\
struct Point { x: Int, y: Int }
fn main() -> Int {
  let p: Point = Point { x: 3, y: 4 };
  return p.x + p.y;
}";
    assert_eq!(check(src), 7);
}

#[test]
fn field_write_mutation() {
    let src = "\
struct Point { x: Int, y: Int }
fn main() -> Int {
  let mut p: Point = Point { x: 1, y: 2 };
  p.x = 10;
  p.y = p.y + 5;
  return p.x + p.y;
}";
    assert_eq!(check(src), 17);
}

#[test]
fn struct_value_semantics_on_assignment() {
    // Copying a struct must be independent: mutating the copy leaves the original.
    let src = "\
struct Box { v: Int }
fn main() -> Int {
  let a: Box = Box { v: 1 };
  let mut b: Box = a;
  b.v = 99;
  return a.v;
}";
    assert_eq!(check(src), 1);
}

#[test]
fn struct_as_param_is_passed_by_value() {
    let src = "\
struct Box { v: Int }
fn bump(b: Box) -> Int { return b.v + 1; }
fn main() -> Int {
  let a: Box = Box { v: 41 };
  return bump(a);
}";
    assert_eq!(check(src), 42);
}

#[test]
fn struct_returned_from_function() {
    let src = "\
struct Point { x: Int, y: Int }
fn make(a: Int, b: Int) -> Point { return Point { x: a, y: b }; }
fn main() -> Int {
  let p: Point = make(10, 20);
  return p.x * p.y;
}";
    assert_eq!(check(src), 200);
}

#[test]
fn nested_struct() {
    let src = "\
struct Inner { n: Int }
struct Outer { inner: Inner, tag: Int }
fn main() -> Int {
  let o: Outer = Outer { inner: Inner { n: 7 }, tag: 100 };
  return o.inner.n + o.tag;
}";
    assert_eq!(check(src), 107);
}

#[test]
fn struct_field_in_loop() {
    let src = "\
struct Acc { sum: Int, count: Int }
fn main() -> Int {
  let mut a: Acc = Acc { sum: 0, count: 0 };
  let mut i: Int = 0;
  while i < 10 {
    a.sum = a.sum + i;
    a.count = a.count + 1;
    i = i + 1;
  }
  return a.sum + a.count;
}";
    assert_eq!(check(src), 45 + 10);
}
