// std.io stdout output: `print` / `println` write to fd 1 via libc `write`.
// The differential harness compares main's Int RETURN (the printed bytes are a
// side effect to the process stdout, not captured here), so these tests confirm
// both backends agree on the return value and neither crashes. Run with
// `-- --nocapture` to eyeball the native JIT's actual output.
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_io -- --nocapture
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
fn print_and_println_return_zero_and_run() {
    // Both backends emit to stdout (visible with --nocapture) and `print` yields 0.
    let src = "\
import std.io;
fn main() -> Int {
  print(\"[codegen_io] hello, \");
  println(\"world\");
  return print(\"\");   // print returns 0
}";
    assert_eq!(check(src), 0);
}

#[test]
fn user_defined_print_shadows_std_builtin() {
    // A user `fn print` must win over the std.io builtin in BOTH backends — this is
    // what lets the bare-metal kernel route `print` to its UART instead of libc
    // `write` (which is unlinkable freestanding). If the builtin shadowed the user
    // function, native `print(..)` would return 0 (the builtin) instead of 5.
    let src = "\
import std.core;
fn print(s: String) -> Int { return string_len(s); }
fn main() -> Int {
  return print(\"hello\");   // user print returns the length, not 0
}";
    assert_eq!(check(src), 5);
}

#[test]
fn print_composed_with_string_builtins() {
    let src = "\
import std.core;
import std.io;
fn main() -> Int {
  let msg: String = string_concat(\"answer = \", int_to_string(42));
  println(msg);
  return string_len(msg);   // len of `answer = 42` is 11
}";
    assert_eq!(check(src), 11);
}
