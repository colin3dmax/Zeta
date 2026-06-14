// Native backend — string subset differential tests (cargo feature `llvm`).
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_string
//
// Strings are immutable `{ i64 len, ptr<i8> data }`: literals, `string_len`,
// `string_byte_at`. Results are observed as Int (the differential oracle, the
// Stage0 interpreter, returns an Int from `main`).
#![cfg(feature = "llvm")]

use zeta::runtime::Value;

fn check(source: &str) -> i64 {
    // `import std.core;` makes the resolver aware of the string_* / int_to_string
    // builtins; codegen lowers them inline (no actual module is linked).
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
fn literal_len() {
    let src = "\
fn main() -> Int {
  let s: String = \"hello\";
  return string_len(s);
}";
    assert_eq!(check(src), 5);
}

#[test]
fn empty_string_len() {
    let src = "\
fn main() -> Int {
  let s: String = \"\";
  return string_len(s);
}";
    assert_eq!(check(src), 0);
}

#[test]
fn byte_at_ascii() {
    // 'A' = 65, 'B' = 66, 'C' = 67
    let src = "\
fn main() -> Int {
  let s: String = \"ABC\";
  return string_byte_at(s, 0) + string_byte_at(s, 2);
}";
    assert_eq!(check(src), 65 + 67);
}

#[test]
fn byte_is_unsigned() {
    // Byte 0xFF must widen to 255 (unsigned), not -1. Use a UTF-8 byte > 127.
    let src = "\
fn main() -> Int {
  let s: String = \"é\";
  return string_byte_at(s, 0);
}";
    // "é" is U+00E9 → UTF-8 bytes 0xC3 0xA9; first byte = 195.
    assert_eq!(check(src), 195);
}

#[test]
fn string_param_and_loop() {
    // Sum all bytes by walking string_len / string_byte_at in a loop.
    let src = "\
fn sum_bytes(s: String) -> Int {
  let mut i: Int = 0;
  let mut total: Int = 0;
  while i < string_len(s) {
    total = total + string_byte_at(s, i);
    i = i + 1;
  }
  return total;
}
fn main() -> Int {
  return sum_bytes(\"AB\");
}";
    assert_eq!(check(src), 65 + 66);
}

#[test]
fn string_returned_from_function() {
    let src = "\
fn greeting() -> String { return \"hi\"; }
fn main() -> Int {
  let s: String = greeting();
  return string_len(s) + string_byte_at(s, 0);
}";
    // len 2 + 'h'(104)
    assert_eq!(check(src), 2 + 104);
}
