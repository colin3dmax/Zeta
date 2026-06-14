// Native backend — scalar subset differential tests (cargo feature `llvm`).
//
// The whole file compiles to nothing unless `--features llvm` is set, so the
// default build/CI needs no LLVM toolchain. Run with:
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_scalar
//
// For every program, the JIT-compiled native result must equal what the Stage0
// interpreter (`run_mir`) produces — the interpreter is the differential oracle.
#![cfg(feature = "llvm")]

use zeta::runtime::Value;

/// Run `fn main() -> Int { ... }` through both the interpreter and the native
/// JIT and assert they agree; returns the (shared) result.
fn check(source: &str) -> i64 {
    let program = zeta::lower_source(source).expect("source should lower");

    let oracle = match zeta::runtime::run_mir(&program).expect("interpreter should run") {
        Value::Int(n) => n,
        other => panic!("expected Int from interpreter, got {other:?}"),
    };
    let native = zeta::codegen::jit_run_i64(&program, "main").expect("native JIT should run");

    assert_eq!(
        native, oracle,
        "native/interpreter divergence\n--- source ---\n{source}\n--- native={native} oracle={oracle} ---"
    );
    oracle
}

#[test]
fn arithmetic_and_precedence() {
    assert_eq!(check("fn main() -> Int { return 2 + 3 * 4; }"), 14);
    assert_eq!(check("fn main() -> Int { return (2 + 3) * 4; }"), 20);
    assert_eq!(check("fn main() -> Int { return 17 / 5; }"), 3);
    assert_eq!(check("fn main() -> Int { return 17 % 5; }"), 2);
    assert_eq!(check("fn main() -> Int { return 0 - 7; }"), -7);
}

#[test]
fn unary_and_bitwise() {
    assert_eq!(check("fn main() -> Int { let x: Int = 5; return -x; }"), -5);
    assert_eq!(check("fn main() -> Int { return 6 & 3; }"), 2);
    assert_eq!(check("fn main() -> Int { return 6 | 1; }"), 7);
    assert_eq!(check("fn main() -> Int { return 6 ^ 3; }"), 5);
    assert_eq!(check("fn main() -> Int { let b: Bool = false; if !b { return 1; } return 0; }"), 1);
}

#[test]
fn locals_and_assignment() {
    assert_eq!(
        check("fn main() -> Int { let mut x: Int = 10; x = x + 5; x = x * 2; return x; }"),
        30
    );
}

#[test]
fn if_else_and_comparisons() {
    let src = "fn main() -> Int { let x: Int = 7; if x > 5 { return 100; } else { return 200; } }";
    assert_eq!(check(src), 100);
    let src2 = "fn main() -> Int { let x: Int = 3; if x > 5 { return 100; } else { return 200; } }";
    assert_eq!(check(src2), 200);
    assert_eq!(check("fn main() -> Int { if 4 == 4 { return 1; } return 0; }"), 1);
    assert_eq!(check("fn main() -> Int { if 4 != 4 { return 1; } return 0; }"), 0);
}

#[test]
fn short_circuit_logical() {
    // a && b
    assert_eq!(
        check("fn main() -> Int { let a: Bool = true; let b: Bool = false; if a && b { return 1; } return 0; }"),
        0
    );
    assert_eq!(
        check("fn main() -> Int { let a: Bool = true; let b: Bool = true; if a && b { return 1; } return 0; }"),
        1
    );
    // a || b
    assert_eq!(
        check("fn main() -> Int { let a: Bool = false; let b: Bool = true; if a || b { return 1; } return 0; }"),
        1
    );
}

#[test]
fn while_loop_sum() {
    let src = "\
fn main() -> Int {
  let mut i: Int = 0;
  let mut sum: Int = 0;
  while i < 100 {
    i = i + 1;
    sum = sum + i;
  }
  return sum;
}";
    assert_eq!(check(src), 5050);
}

#[test]
fn break_and_continue() {
    // break: stop summing at i==5
    let brk = "\
fn main() -> Int {
  let mut i: Int = 0;
  let mut sum: Int = 0;
  while i < 100 {
    i = i + 1;
    if i == 5 { break; }
    sum = sum + i;
  }
  return sum;
}";
    assert_eq!(check(brk), 1 + 2 + 3 + 4);

    // continue: skip even i
    let cont = "\
fn main() -> Int {
  let mut i: Int = 0;
  let mut sum: Int = 0;
  while i < 10 {
    i = i + 1;
    if i % 2 == 0 { continue; }
    sum = sum + i;
  }
  return sum;
}";
    assert_eq!(check(cont), 1 + 3 + 5 + 7 + 9);
}

#[test]
fn user_function_calls_and_recursion() {
    let fact = "\
fn fact(n: Int) -> Int { if n <= 1 { return 1; } return n * fact(n - 1); }
fn main() -> Int { return fact(10); }";
    assert_eq!(check(fact), 3628800);

    let fib = "\
fn fib(n: Int) -> Int { if n < 2 { return n; } return fib(n - 1) + fib(n - 2); }
fn main() -> Int { return fib(20); }";
    assert_eq!(check(fib), 6765);
}

#[test]
fn nested_control_flow() {
    let src = "\
fn main() -> Int {
  let mut total: Int = 0;
  let mut i: Int = 0;
  while i < 5 {
    let mut j: Int = 0;
    while j < 5 {
      if (i + j) % 2 == 0 { total = total + 1; }
      j = j + 1;
    }
    i = i + 1;
  }
  return total;
}";
    assert_eq!(check(src), 13);
}
