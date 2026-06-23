// Native backend — `match` on a String scrutinee (cargo feature `llvm`). String
// patterns can't drive an integer switch, so codegen lowers them as a sequential
// chain of `string_eq` tests. Each program runs through BOTH the interpreter
// (oracle) and the native JIT; results must match.
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_match
#![cfg(feature = "llvm")]

use zeta::runtime::Value;

fn check(source: &str) -> i64 {
    let program = zeta::lower_source(source).expect("lower");
    let oracle = match zeta::runtime::run_mir(&program).expect("interpreter") {
        Value::Int(n) => n,
        other => panic!("expected Int, got {other:?}"),
    };
    let native = zeta::codegen::jit_run_i64(&program, &[], "main").expect("native JIT");
    assert_eq!(native, oracle, "native/interpreter divergence\n{source}");
    oracle
}

#[test]
fn string_match_selects_arm() {
    let src = "\
fn classify(s: String) -> Int {
  match s {
    \"yes\" -> { return 1; },
    \"no\"  -> { return 0; },
    _      -> { return 0 - 1; },
  }
}
fn main() -> Int {
  return classify(\"yes\") * 100 + classify(\"no\") * 10 + classify(\"maybe\");
}";
    // 1*100 + 0*10 + (-1) = 99
    assert_eq!(check(src), 99);
}

#[test]
fn string_match_catch_all_binds() {
    // The catch-all binds the scrutinee; first-match-wins order is preserved.
    let src = "\
import std.core;
fn pick(s: String) -> Int {
  match s {
    \"\"   -> { return 0; },
    \"hi\" -> { return 2; },
    other -> { return string_len(other); },
  }
}
fn main() -> Int {
  return pick(\"hi\") + pick(\"hello\") + pick(\"\");   // 2 + 5 + 0 = 7
}";
    assert_eq!(check(src), 7);
}

#[test]
fn string_match_in_loop() {
    // Exercise the sequential chain repeatedly (refcount/drop of the scrutinee).
    let src = "\
import std.core;
fn score(s: String) -> Int {
  match s { \"a\" -> { return 1; }, \"bb\" -> { return 2; }, _ -> { return 0; }, }
}
fn main() -> Int {
  let mut total: Int = 0;
  let mut i: Int = 0;
  while i < 10 {
    total = total + score(\"a\") + score(\"bb\") + score(\"zzz\");
    i = i + 1;
  }
  return total;   // 10 * (1 + 2 + 0) = 30
}";
    assert_eq!(check(src), 30);
}
