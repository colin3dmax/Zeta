// Method-call syntax `recv.method(args)` — pure sugar for `method(recv, args)`,
// reusing UFCS / trait dispatch. Each program runs through BOTH the interpreter
// (oracle) and the native JIT; results must match.
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_method
#![cfg(feature = "llvm")]

use zeta::ast::Item;
use zeta::runtime::Value;

fn check(source: &str) -> i64 {
    let structs: Vec<zeta::ast::StructDecl> = zeta::parse_source(source)
        .expect("parse")
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Struct(d) => Some(d.clone()),
            _ => None,
        })
        .collect();
    let program = zeta::lower_source(source).expect("lower");
    let oracle = match zeta::runtime::run_mir(&program).expect("interpreter") {
        Value::Int(n) => n,
        other => panic!("expected Int, got {other:?}"),
    };
    let native = zeta::codegen::jit_run_i64(&program, &structs, "main").expect("native JIT");
    assert_eq!(native, oracle, "native/interpreter divergence\n{source}");
    oracle
}

#[test]
fn method_call_free_function() {
    // `p.scaled(10)` desugars to `scaled(p, 10)`.
    let src = "\
struct Point { x: Int, y: Int }
fn scaled(p: Point, k: Int) -> Int { return (p.x + p.y) * k; }
fn main() -> Int {
  let p: Point = Point { x: 3, y: 4 };
  return p.scaled(10);
}";
    assert_eq!(check(src), 70);
}

#[test]
fn method_call_trait_dispatch() {
    // `p.show()` routes through trait dispatch to `show$Point`.
    let src = "\
trait Show { fn show(self: Self) -> Int; }
struct Point { x: Int, y: Int }
impl Show for Point { fn show(self: Self) -> Int { return self.x + self.y; } }
fn main() -> Int {
  let p: Point = Point { x: 40, y: 2 };
  return p.show();
}";
    assert_eq!(check(src), 42);
}

#[test]
fn method_call_chained() {
    // `p.show().twice()` = `twice(show(p))`, left to right.
    let src = "\
trait Show { fn show(self: Self) -> Int; }
struct Point { x: Int, y: Int }
impl Show for Point { fn show(self: Self) -> Int { return self.x + self.y; } }
fn twice(n: Int) -> Int { return n * 2; }
fn main() -> Int {
  let p: Point = Point { x: 3, y: 4 };
  return p.show().twice();
}";
    assert_eq!(check(src), 14);
}

#[test]
fn method_call_does_not_break_field_access_or_tuple_index() {
    // `.field` (no parens) stays a field access; `.0` stays a tuple index.
    let src = "\
struct Point { x: Int, y: Int }
fn main() -> Int {
  let p: Point = Point { x: 5, y: 9 };
  let t: (Int, Int) = (11, 22);
  return p.x + p.y + t.0 + t.1;   // 5 + 9 + 11 + 22 = 47
}";
    assert_eq!(check(src), 47);
}

#[test]
fn method_call_on_field_and_index_receiver() {
    // The receiver can be any expression — a field read, an array element, etc.
    let src = "\
import std.core;
struct Wrap { p: Int }
fn doubled(n: Int) -> Int { return n * 2; }
fn main() -> Int {
  let w: Wrap = Wrap { p: 21 };
  let xs: IntArray = [3, 4, 5];
  return w.p.doubled() + xs[1].doubled();   // 42 + 8 = 50
}";
    assert_eq!(check(src), 50);
}
