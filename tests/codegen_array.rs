// Native backend — array subset differential tests (cargo feature `llvm`).
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_array
//
// IntArray with value semantics: literals, indexing (read/write), `.len`, arrays
// in loops, and copy-independence (a copy mutated does not affect the original).
// The Stage0 interpreter is the differential oracle.
#![cfg(feature = "llvm")]

use zeta::runtime::Value;

fn check(source: &str) -> i64 {
    let program = zeta::lower_source(source).expect("should lower");
    let oracle = match zeta::runtime::run_mir(&program).expect("interpreter should run") {
        Value::Int(n) => n,
        other => panic!("expected Int, got {other:?}"),
    };
    // No struct decls in these programs.
    let native = zeta::codegen::jit_run_i64(&program, &[], "main").expect("native JIT");
    assert_eq!(
        native, oracle,
        "native/interpreter divergence\n--- source ---\n{source}\n--- native={native} oracle={oracle} ---"
    );
    oracle
}

#[test]
fn literal_index_and_len() {
    let src = "\
fn main() -> Int {
  let xs: IntArray = [10, 20, 30];
  return xs[0] + xs[2] + xs.len;
}";
    assert_eq!(check(src), 10 + 30 + 3);
}

#[test]
fn index_write() {
    let src = "\
fn main() -> Int {
  let mut xs: IntArray = [1, 2, 3];
  xs[1] = 99;
  return xs[0] + xs[1] + xs[2];
}";
    assert_eq!(check(src), 1 + 99 + 3);
}

#[test]
fn sum_over_array_in_loop() {
    let src = "\
fn main() -> Int {
  let xs: IntArray = [5, 10, 15, 20, 25];
  let mut i: Int = 0;
  let mut sum: Int = 0;
  while i < xs.len {
    sum = sum + xs[i];
    i = i + 1;
  }
  return sum;
}";
    assert_eq!(check(src), 75);
}

#[test]
fn value_semantics_copy_is_independent() {
    // `let b = a` must deep-copy: mutating b leaves a unchanged.
    let src = "\
fn main() -> Int {
  let a: IntArray = [1, 2, 3];
  let mut b: IntArray = a;
  b[0] = 100;
  return a[0] + b[0];
}";
    // a[0] stays 1, b[0] becomes 100.
    assert_eq!(check(src), 101);
}

#[test]
fn array_passed_to_function_by_value() {
    let src = "\
fn first(xs: IntArray) -> Int { return xs[0]; }
fn mutate_local(xs: IntArray) -> Int {
  let mut ys: IntArray = xs;
  ys[0] = 999;
  return ys[0];
}
fn main() -> Int {
  let a: IntArray = [7, 8, 9];
  let inside: Int = mutate_local(a);
  // a must be untouched by mutate_local (pass-by-value + local copy).
  return first(a) + inside;
}";
    assert_eq!(check(src), 7 + 999);
}

#[test]
fn array_returned_from_function() {
    let src = "\
fn build(a: Int, b: Int) -> IntArray { return [a, b, a + b]; }
fn main() -> Int {
  let xs: IntArray = build(3, 4);
  return xs[0] + xs[1] + xs[2];
}";
    assert_eq!(check(src), 3 + 4 + 7);
}

#[test]
fn write_then_grow_sum() {
    let src = "\
fn main() -> Int {
  let mut xs: IntArray = [0, 0, 0, 0];
  let mut i: Int = 0;
  while i < xs.len {
    xs[i] = i * i;
    i = i + 1;
  }
  return xs[0] + xs[1] + xs[2] + xs[3];
}";
    assert_eq!(check(src), 0 + 1 + 4 + 9);
}
