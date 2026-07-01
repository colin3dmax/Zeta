// Trait default methods: a `trait` may give a method a body, which an `impl`
// inherits unless it provides its own. Implemented by synthesizing the same
// mangled `method$Base` free function from the default body (Self → target) for
// each impl that omits the method. These tests confirm the interpreter (oracle)
// and native JIT agree, across override and inherit.
#![cfg(feature = "llvm")]

use zeta::ast::Item;
use zeta::runtime::Value;

fn check(source: &str) -> i64 {
    let module = zeta::parse_source(source).expect("parse");
    let structs: Vec<zeta::ast::StructDecl> = module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(decl) => Some(decl.clone()),
            _ => None,
        })
        .collect();
    let program = zeta::lower_source(source).expect("lower");
    let oracle = match zeta::runtime::run_mir(&program).expect("interpreter") {
        Value::Int(n) => n,
        other => panic!("expected Int, got {other:?}"),
    };
    let native = zeta::codegen::jit_run_i64(&program, &structs, "main").expect("native JIT");
    assert_eq!(native, oracle, "native/interpreter divergence");
    oracle
}

#[test]
fn default_inherited_and_overridden() {
    // Cat overrides `greeting`; Dog inherits the default, which itself calls the
    // required `name` method on self (UFCS dispatch after mangling).
    let src = "\
import std.core;
trait Greet {
  fn name(self: Self) -> String;
  fn greeting(self: Self) -> String { return string_concat(\"Hello, \", name(self)); }
}
struct Cat { legs: Int }
struct Dog { legs: Int }
impl Greet for Cat {
  fn name(self: Self) -> String { return \"Cat\"; }
  fn greeting(self: Self) -> String { return \"Meow\"; }
}
impl Greet for Dog {
  fn name(self: Self) -> String { return \"Dog\"; }
}
fn main() -> Int {
  let c: Cat = Cat { legs: 4 };
  let d: Dog = Dog { legs: 4 };
  return string_len(greeting(c)) * 100 + string_len(greeting(d));
}";
    // Cat -> "Meow" (4); Dog -> "Hello, Dog" (10). 4*100 + 10 = 410.
    assert_eq!(check(src), 410);
}

#[test]
fn default_computes_from_required_int_method() {
    // A default that combines results of required methods; both impls inherit it.
    let src = "\
trait Metric {
  fn base(self: Self) -> Int;
  fn factor(self: Self) -> Int;
  fn score(self: Self) -> Int { return base(self) * factor(self); }
}
struct A { v: Int }
struct B { v: Int }
impl Metric for A {
  fn base(self: Self) -> Int { return 3; }
  fn factor(self: Self) -> Int { return 4; }
}
impl Metric for B {
  fn base(self: Self) -> Int { return 10; }
  fn factor(self: Self) -> Int { return 2; }
  fn score(self: Self) -> Int { return 999; }
}
fn main() -> Int {
  let a: A = A { v: 0 };
  let b: B = B { v: 0 };
  return score(a) * 1000 + score(b);   // A default 12 -> 12000; B override 999. 12999
}";
    assert_eq!(check(src), 12999);
}

#[test]
fn default_used_by_trait_bound_generic() {
    // A `<T: Metric>` generic calls a defaulted method — the synthesized
    // `score$T` must exist so the bound is satisfied and dispatch resolves.
    let src = "\
trait Metric {
  fn base(self: Self) -> Int;
  fn score(self: Self) -> Int { return base(self) + 1; }
}
struct P { v: Int }
impl Metric for P {
  fn base(self: Self) -> Int { return 41; }
}
fn scored<T: Metric>(x: T) -> Int { return score(x); }
fn main() -> Int {
  let p: P = P { v: 0 };
  return scored(p);   // default score = base + 1 = 42
}";
    assert_eq!(check(src), 42);
}
