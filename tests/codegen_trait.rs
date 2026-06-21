// Native backend — trait / impl UFCS dispatch differential tests (feature `llvm`).
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_trait
//
// A call to a trait method `m(recv, ..)` dispatches by the receiver's concrete
// type to the flattened impl `m$<TypeBase>`. For every program the JIT-compiled
// native result must equal the Stage0 interpreter's (the differential oracle).
#![cfg(feature = "llvm")]

use zeta::ast::Item;
use zeta::runtime::Value;

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
fn dispatches_by_struct_type() {
    // show(p) → Point's impl, show(c) → Circle's impl. 3+4 + 5*5 = 32.
    let result = check(
        r#"
trait Show {
  fn show(self: Self) -> Int;
}
struct Point { x: Int, y: Int }
struct Circle { r: Int }
impl Show for Point {
  fn show(self: Self) -> Int { return self.x + self.y; }
}
impl Show for Circle {
  fn show(self: Self) -> Int { return self.r * self.r; }
}
fn main() -> Int {
  let p: Point = Point { x: 3, y: 4 };
  let c: Circle = Circle { r: 5 };
  return show(p) + show(c);
}
"#,
    );
    assert_eq!(result, 32);
}

#[test]
fn dispatches_on_scalar_and_struct_with_extra_param() {
    // scaled(n, 10) → Int impl = 6*10; scaled(v, 100) → Vec2 impl = (1+2)*100.
    let result = check(
        r#"
trait Scale {
  fn scaled(self: Self, factor: Int) -> Int;
}
impl Scale for Int {
  fn scaled(self: Self, factor: Int) -> Int { return self * factor; }
}
struct Vec2 { x: Int, y: Int }
impl Scale for Vec2 {
  fn scaled(self: Self, factor: Int) -> Int { return (self.x + self.y) * factor; }
}
fn main() -> Int {
  let n: Int = 6;
  let v: Vec2 = Vec2 { x: 1, y: 2 };
  return scaled(n, 10) + scaled(v, 100);
}
"#,
    );
    assert_eq!(result, 360);
}

#[test]
fn dispatches_on_enum_receiver() {
    let result = check(
        r#"
trait Weight {
  fn weight(self: Self) -> Int;
}
enum Shape { Dot, Line(Int) }
impl Weight for Shape {
  fn weight(self: Self) -> Int {
    match self {
      Shape.Dot -> { return 1; }
      Shape.Line(n) -> { return n; }
    }
  }
}
fn main() -> Int {
  let a: Shape = Shape.Dot;
  let b: Shape = Shape.Line(40);
  return weight(a) + weight(b);
}
"#,
    );
    assert_eq!(result, 41);
}

#[test]
fn trait_method_calls_another_trait_method() {
    // A trait method body can itself dispatch on its receiver's field.
    let result = check(
        r#"
trait Area {
  fn area(self: Self) -> Int;
}
struct Rect { w: Int, h: Int }
impl Area for Rect {
  fn area(self: Self) -> Int { return self.w * self.h; }
}
fn total(r: Rect) -> Int {
  return area(r) + area(r);
}
fn main() -> Int {
  let r: Rect = Rect { w: 3, h: 7 };
  return total(r);
}
"#,
    );
    assert_eq!(result, 42);
}
