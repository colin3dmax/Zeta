// Native backend — `for` loop subset differential tests (cargo feature `llvm`).
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_for
//
// `for i in a..b` (exclusive end), `for x in intArray`, and C-style
// `for (init; cond; step)`, including break/continue. The Stage0 interpreter is
// the differential oracle.
#![cfg(feature = "llvm")]

use zeta::runtime::Value;

fn check(source: &str) -> i64 {
    let program = zeta::lower_source(source).expect("should lower");
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
fn range_sum() {
    let src = "\
fn main() -> Int {
  let mut sum: Int = 0;
  for i in 0..5 {
    sum = sum + i;
  }
  return sum;
}";
    // 0+1+2+3+4
    assert_eq!(check(src), 10);
}

#[test]
fn range_empty_when_start_ge_end() {
    let src = "\
fn main() -> Int {
  let mut sum: Int = 0;
  for i in 5..5 {
    sum = sum + 1;
  }
  for i in 8..3 {
    sum = sum + 1;
  }
  return sum;
}";
    assert_eq!(check(src), 0);
}

#[test]
fn range_bounds_evaluated_once() {
    // The end bound is a variable that the body mutates; the loop must use the
    // original value (bounds evaluated once up front), matching the interpreter.
    let src = "\
fn main() -> Int {
  let mut n: Int = 3;
  let mut count: Int = 0;
  for i in 0..n {
    count = count + 1;
    n = n + 10;
  }
  return count;
}";
    assert_eq!(check(src), 3);
}

#[test]
fn range_with_break() {
    let src = "\
fn main() -> Int {
  let mut sum: Int = 0;
  for i in 0..100 {
    if i == 5 { break; }
    sum = sum + i;
  }
  return sum;
}";
    // 0+1+2+3+4
    assert_eq!(check(src), 10);
}

#[test]
fn range_with_continue_still_increments() {
    // `continue` must still advance the counter (latch runs), else infinite loop.
    let src = "\
fn main() -> Int {
  let mut sum: Int = 0;
  for i in 0..10 {
    if i == 3 { continue; }
    sum = sum + i;
  }
  return sum;
}";
    // 45 - 3
    assert_eq!(check(src), 42);
}

#[test]
fn nested_range_loops() {
    let src = "\
fn main() -> Int {
  let mut total: Int = 0;
  for i in 0..3 {
    for j in 0..3 {
      total = total + i * j;
    }
  }
  return total;
}";
    // sum over i,j of i*j = (0+1+2)*(0+1+2) = 9
    assert_eq!(check(src), 9);
}

#[test]
fn for_in_array_sum() {
    let src = "\
fn main() -> Int {
  let xs: IntArray = [3, 1, 4, 1, 5, 9];
  let mut sum: Int = 0;
  for x in xs {
    sum = sum + x;
  }
  return sum;
}";
    assert_eq!(check(src), 3 + 1 + 4 + 1 + 5 + 9);
}

#[test]
fn for_in_array_with_break_continue() {
    let src = "\
fn main() -> Int {
  let xs: IntArray = [10, 20, 30, 40, 50];
  let mut sum: Int = 0;
  for x in xs {
    if x == 20 { continue; }
    if x == 40 { break; }
    sum = sum + x;
  }
  return sum;
}";
    // 10 + (skip 20) + 30 + (break at 40)
    assert_eq!(check(src), 40);
}

#[test]
fn for_in_array_then_range() {
    // for-in followed by a range loop in the same function (distinct loop var
    // slots, both latches advance independently).
    let src = "\
fn main() -> Int {
  let xs: IntArray = [2, 4, 6];
  let mut sum: Int = 0;
  for x in xs {
    sum = sum + x;
  }
  for i in 0..4 {
    sum = sum + i;
  }
  return sum;
}";
    // (2+4+6) + (0+1+2+3)
    assert_eq!(check(src), 12 + 6);
}

#[test]
fn c_style_for_sum() {
    let src = "\
fn main() -> Int {
  let mut sum: Int = 0;
  for (let mut i: Int = 0; i < 5; i = i + 1) {
    sum = sum + i;
  }
  return sum;
}";
    assert_eq!(check(src), 10);
}

#[test]
fn c_style_for_continue_runs_step() {
    // `continue` must run the step (i = i + 1), else infinite loop.
    let src = "\
fn main() -> Int {
  let mut sum: Int = 0;
  for (let mut i: Int = 0; i < 10; i = i + 1) {
    if i == 4 { continue; }
    sum = sum + i;
  }
  return sum;
}";
    // 45 - 4
    assert_eq!(check(src), 41);
}

#[test]
fn c_style_for_with_break() {
    let src = "\
fn main() -> Int {
  let mut product: Int = 1;
  for (let mut i: Int = 1; i < 100; i = i + 1) {
    if i > 5 { break; }
    product = product * i;
  }
  return product;
}";
    // 1*2*3*4*5
    assert_eq!(check(src), 120);
}
