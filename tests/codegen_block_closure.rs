// Block-body closures: `|params| { ...; return e; }`. The block runs like a
// function body (explicit `return`), can declare locals, and captures enclosing
// variables by value — same as the single-expression form. These tests confirm
// the interpreter (oracle) and native JIT agree across block-closure shapes.
#![cfg(feature = "llvm")]

use zeta::runtime::Value;

fn check(source: &str) -> i64 {
    let program = zeta::lower_source(source).expect("lower");
    let oracle = match zeta::runtime::run_mir(&program).expect("interpreter") {
        Value::Int(n) => n,
        other => panic!("expected Int, got {other:?}"),
    };
    let native = zeta::codegen::jit_run_i64(&program, &[], "main").expect("native JIT");
    assert_eq!(native, oracle, "native/interpreter divergence");
    oracle
}

#[test]
fn block_closure_locals_and_capture() {
    // Local `let`s inside the block + a captured outer variable.
    let src = "\
fn apply(f: fn(Int) -> Int, x: Int) -> Int { return f(x); }
fn main() -> Int {
  let factor: Int = 10;
  let g: fn(Int) -> Int = |n| {
    let doubled: Int = n * 2;
    let scaled: Int = doubled * factor;
    return scaled;
  };
  let h: fn(Int) -> Int = |n| n + 1;   // single-expr form still works
  return apply(g, 5) + apply(h, 100);  // 100 + 101 = 201
}";
    assert_eq!(check(src), 201);
}

#[test]
fn block_closure_with_control_flow() {
    // `if`/`return` inside the block body.
    let src = "\
fn apply(f: fn(Int) -> Int, x: Int) -> Int { return f(x); }
fn main() -> Int {
  let g: fn(Int) -> Int = |n| {
    if n > 0 {
      return n * n;
    }
    return 0 - n;
  };
  return apply(g, 6) * 100 + apply(g, 0 - 4);   // 36*100 + 4 = 3604
}";
    assert_eq!(check(src), 3604);
}

#[test]
fn block_closure_with_loop() {
    // A loop accumulator inside a block closure, capturing the bound.
    let src = "\
fn apply(f: fn(Int) -> Int, x: Int) -> Int { return f(x); }
fn main() -> Int {
  let bound: Int = 5;
  let sum_to: fn(Int) -> Int = |start| {
    let mut acc: Int = 0;
    let mut i: Int = start;
    while i <= bound {
      acc = acc + i;
      i = i + 1;
    }
    return acc;
  };
  return apply(sum_to, 1);   // 1+2+3+4+5 = 15
}";
    assert_eq!(check(src), 15);
}

#[test]
fn block_closure_returns_string() {
    // A block closure whose return type is a managed value (String).
    let src = "\
import std.core;
fn apply(f: fn(Int) -> String, x: Int) -> String { return f(x); }
fn main() -> Int {
  let prefix: String = \"n=\";
  let label: fn(Int) -> String = |n| {
    let s: String = string_concat(prefix, int_to_string(n));
    return s;
  };
  return string_len(apply(label, 42));   // \"n=42\" has length 4
}";
    assert_eq!(check(src), 4);
}

#[test]
fn block_closure_multiple_captures() {
    let src = "\
fn apply2(f: fn(Int, Int) -> Int, a: Int, b: Int) -> Int { return f(a, b); }
fn main() -> Int {
  let base: Int = 1000;
  let mul: Int = 3;
  let combine: fn(Int, Int) -> Int = |x, y| {
    let t: Int = x * mul + y;
    return base + t;
  };
  return apply2(combine, 5, 7);   // 1000 + (5*3 + 7) = 1022
}";
    assert_eq!(check(src), 1022);
}
