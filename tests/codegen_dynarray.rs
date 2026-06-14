// Native backend — growable IntArray tests (cargo feature `llvm`).
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_dynarray
//
// `int_array_empty()` + `int_array_push(arr, x)` (functional append). The Stage0
// interpreter is the differential oracle.
#![cfg(feature = "llvm")]

use zeta::runtime::Value;

fn check(source: &str) -> i64 {
    let source = format!("import std.core;\n{source}");
    let program = zeta::lower_source(&source).expect("should lower");
    let oracle = match zeta::runtime::run_mir(&program).expect("interpreter should run") {
        Value::Int(n) => n,
        other => panic!("expected Int, got {other:?}"),
    };
    let native = zeta::codegen::jit_run_i64(&program, &[], "main").expect("native JIT");
    assert_eq!(
        native, oracle,
        "native/interpreter divergence\n--- source ---\n{source}\n--- native={native} oracle={oracle} ---"
    );
    oracle
}

#[test]
fn empty_has_zero_len() {
    let src = "\
fn main() -> Int {
  let xs: IntArray = int_array_empty();
  return xs.len;
}";
    assert_eq!(check(src), 0);
}

#[test]
fn push_one_then_read() {
    let src = "\
fn main() -> Int {
  let xs: IntArray = int_array_push(int_array_empty(), 42);
  return xs.len * 1000 + xs[0];
}";
    // len 1, xs[0] = 42
    assert_eq!(check(src), 1042);
}

#[test]
fn push_in_loop_and_sum() {
    let src = "\
fn main() -> Int {
  let mut xs: IntArray = int_array_empty();
  for i in 0..6 {
    xs = int_array_push(xs, i * i);
  }
  let mut sum: Int = 0;
  for x in xs {
    sum = sum + x;
  }
  return sum * 100 + xs.len;
}";
    // squares 0..5: 0+1+4+9+16+25 = 55, len 6
    assert_eq!(check(src), 55 * 100 + 6);
}

#[test]
fn push_preserves_order() {
    let src = "\
fn main() -> Int {
  let mut xs: IntArray = int_array_empty();
  xs = int_array_push(xs, 7);
  xs = int_array_push(xs, 8);
  xs = int_array_push(xs, 9);
  return xs[0] * 100 + xs[1] * 10 + xs[2];
}";
    assert_eq!(check(src), 789);
}

#[test]
fn push_is_functional_original_untouched() {
    // `int_array_push` returns a new array; the source array is unchanged. This is
    // the value-semantics contract (the interpreter's copy-on-write push).
    let src = "\
fn main() -> Int {
  let a: IntArray = int_array_push(int_array_empty(), 1);
  let b: IntArray = int_array_push(a, 2);
  // a still has length 1; b has length 2.
  return a.len * 1000 + b.len * 100 + a[0] + b[1];
}";
    // a.len=1, b.len=2, a[0]=1, b[1]=2 → 1000 + 200 + 1 + 2
    assert_eq!(check(src), 1203);
}

#[test]
fn push_onto_literal() {
    let src = "\
fn main() -> Int {
  let xs: IntArray = int_array_push([10, 20], 30);
  return xs.len * 1000 + xs[0] + xs[1] + xs[2];
}";
    // len 3, 10+20+30
    assert_eq!(check(src), 3 * 1000 + 60);
}

#[test]
fn build_then_index_write() {
    // Build dynamically, then mutate an element in place (value semantics: the
    // built buffer is exclusively owned).
    let src = "\
fn main() -> Int {
  let mut xs: IntArray = int_array_empty();
  for i in 0..4 {
    xs = int_array_push(xs, i);
  }
  xs[2] = 99;
  return xs[0] + xs[1] + xs[2] + xs[3];
}";
    // 0 + 1 + 99 + 3
    assert_eq!(check(src), 103);
}

#[test]
fn push_to_function_built_array() {
    let src = "\
fn build(n: Int) -> IntArray {
  let mut xs: IntArray = int_array_empty();
  for i in 0..n {
    xs = int_array_push(xs, i + 1);
  }
  return xs;
}
fn main() -> Int {
  let xs: IntArray = build(5);
  let mut sum: Int = 0;
  for x in xs {
    sum = sum + x;
  }
  return sum;
}";
    // 1+2+3+4+5
    assert_eq!(check(src), 15);
}
