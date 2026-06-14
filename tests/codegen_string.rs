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

#[test]
fn concat_len() {
    let src = "\
fn main() -> Int {
  let s: String = string_concat(\"foo\", \"barbaz\");
  return string_len(s);
}";
    assert_eq!(check(src), 9);
}

#[test]
fn concat_bytes_in_order() {
    // "AB" ++ "CD" → bytes A,B,C,D at 0..3
    let src = "\
fn main() -> Int {
  let s: String = string_concat(\"AB\", \"CD\");
  return string_byte_at(s, 0) * 1000 + string_byte_at(s, 1) * 100
       + string_byte_at(s, 2) * 10 + string_byte_at(s, 3);
}";
    // 65,66,67,68
    assert_eq!(check(src), 65 * 1000 + 66 * 100 + 67 * 10 + 68);
}

#[test]
fn concat_with_empty() {
    let src = "\
fn main() -> Int {
  let s: String = string_concat(\"\", \"xyz\");
  return string_len(s) + string_byte_at(s, 0);
}";
    // len 3 + 'x'(120)
    assert_eq!(check(src), 3 + 120);
}

#[test]
fn byte_slice_basic() {
    // "hello"[1..4] = "ell"
    let src = "\
fn main() -> Int {
  let s: String = string_byte_slice(\"hello\", 1, 3);
  return string_len(s) * 1000 + string_byte_at(s, 0);
}";
    // len 3, first byte 'e'(101)
    assert_eq!(check(src), 3 * 1000 + 101);
}

#[test]
fn slice_then_concat_chain() {
    // Mirrors run_std_builtin_chain.zeta's shape: slice + concat composed.
    let src = "\
fn main() -> Int {
  let text: String = \"zeta-lang\";
  let head: String = string_byte_slice(text, 0, 4);
  let s: String = string_concat(head, \"!\");
  return string_len(s) * 100 + string_byte_at(s, 4);
}";
    // head = "zeta"(4) ++ "!"(1) → len 5, byte[4] = '!'(33)
    assert_eq!(check(src), 5 * 100 + 33);
}

#[test]
fn string_eq_equal_and_unequal() {
    let src = "\
fn eq(a: String, b: String) -> Int { if a == b { return 1; } return 0; }
fn main() -> Int {
  // equal; different length; same length different bytes; empty==empty
  return eq(\"abc\", \"abc\") * 1000
       + eq(\"abc\", \"ab\") * 100
       + eq(\"abc\", \"abd\") * 10
       + eq(\"\", \"\");
}";
    // 1, 0, 0, 1
    assert_eq!(check(src), 1001);
}

#[test]
fn string_neq() {
    let src = "\
fn main() -> Int {
  let s: String = string_concat(\"foo\", \"bar\");
  if s != \"foobar\" { return 7; }
  if s != \"foobaz\" { return 9; }
  return 0;
}";
    // s == \"foobar\" so first != is false; s != \"foobaz\" is true → 9
    assert_eq!(check(src), 9);
}

#[test]
fn string_eq_after_concat_dynamic() {
    let src = "\
fn main() -> Int {
  let a: String = string_concat(int_to_string(12), int_to_string(34));
  if a == \"1234\" { return 42; }
  return 0;
}";
    assert_eq!(check(src), 42);
}

#[test]
fn int_to_string_zero() {
    let src = "\
fn main() -> Int {
  let s: String = int_to_string(0);
  return string_len(s) * 1000 + string_byte_at(s, 0);
}";
    // "0" → len 1, byte '0'(48)
    assert_eq!(check(src), 1 * 1000 + 48);
}

#[test]
fn int_to_string_multi_digit() {
    let src = "\
fn main() -> Int {
  let s: String = int_to_string(42);
  return string_len(s) * 10000 + string_byte_at(s, 0) * 100 + string_byte_at(s, 1);
}";
    // "42" → len 2, '4'(52), '2'(50)
    assert_eq!(check(src), 2 * 10000 + 52 * 100 + 50);
}

#[test]
fn int_to_string_negative() {
    let src = "\
fn main() -> Int {
  let s: String = int_to_string(0 - 7);
  return string_len(s) * 10000 + string_byte_at(s, 0) * 100 + string_byte_at(s, 1);
}";
    // "-7" → len 2, '-'(45), '7'(55)
    assert_eq!(check(src), 2 * 10000 + 45 * 100 + 55);
}

#[test]
fn int_to_string_large_roundtrip_len() {
    let src = "\
fn main() -> Int {
  let s: String = int_to_string(1234567);
  return string_len(s);
}";
    assert_eq!(check(src), 7);
}

#[test]
fn int_to_string_then_byte_at_each() {
    // Sum all digit bytes of "1234567".
    let src = "\
fn main() -> Int {
  let s: String = int_to_string(1234567);
  let mut i: Int = 0;
  let mut total: Int = 0;
  while i < string_len(s) {
    total = total + string_byte_at(s, i);
    i = i + 1;
  }
  return total;
}";
    // bytes '1'..'7' = 49+50+51+52+53+54+55
    assert_eq!(check(src), 49 + 50 + 51 + 52 + 53 + 54 + 55);
}

// --- ascii predicates (Bool → reified to Int via if/else) ---

#[test]
fn ascii_is_digit_cases() {
    let src = "\
fn b(c: Int) -> Int { if ascii_is_digit(c) { return 1; } return 0; }
fn main() -> Int {
  // '0'(48) yes, '9'(57) yes, 'A'(65) no, 47 no, 58 no, -1 no, 300 no
  return b(48) * 1000000 + b(57) * 100000 + b(65) * 10000
       + b(47) * 1000 + b(58) * 100 + b(0 - 1) * 10 + b(300);
}";
    assert_eq!(check(src), 1_100_000);
}

#[test]
fn ascii_is_alpha_cases() {
    let src = "\
fn b(c: Int) -> Int { if ascii_is_alpha(c) { return 1; } return 0; }
fn main() -> Int {
  // 'A'(65) 'Z'(90) 'a'(97) 'z'(122) yes; '0'(48) no; 64 no; 91 no
  return b(65) * 1000000 + b(90) * 100000 + b(97) * 10000
       + b(122) * 1000 + b(48) * 100 + b(64) * 10 + b(91);
}";
    assert_eq!(check(src), 1_111_000);
}

#[test]
fn ascii_is_alnum_and_whitespace() {
    let src = "\
fn an(c: Int) -> Int { if ascii_is_alnum(c) { return 1; } return 0; }
fn ws(c: Int) -> Int { if ascii_is_whitespace(c) { return 1; } return 0; }
fn main() -> Int {
  // alnum: '5'(53) yes, 'q'(113) yes, '+'(43) no
  // whitespace: ' '(32) yes, tab(9) yes, LF(10) yes, vtab(11) NO, 'x'(120) no
  return an(53) * 100000000 + an(113) * 10000000 + an(43) * 1000000
       + ws(32) * 100000 + ws(9) * 10000 + ws(10) * 1000
       + ws(11) * 100 + ws(120) * 10;
}";
    // an: 1,1,0 ; ws: 1,1,1,0,0
    assert_eq!(check(src), 110_111_000);
}
