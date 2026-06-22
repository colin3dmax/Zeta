// Native backend — new std builtins (int_abs/min/max, string_starts_with/
// index_of/contains/repeat). Each program runs through BOTH the interpreter
// (oracle) and the native JIT; the Int results must match.
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_stdlib
#![cfg(feature = "llvm")]

use zeta::ast::Item;
use zeta::runtime::Value;

fn check(source: &str) -> i64 {
    let structs: Vec<zeta::ast::StructDecl> = zeta::parse_source(source)
        .expect("parse")
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
    assert_eq!(native, oracle, "native/interpreter divergence\n{source}");
    oracle
}

fn prog(body: &str) -> String {
    format!("import std.core;\nfn main() -> Int {{\n{body}\n}}")
}

#[test]
fn int_abs_min_max() {
    assert_eq!(check(&prog("  return int_abs(0 - 5) + int_abs(7);")), 12);
    assert_eq!(check(&prog("  return int_min(3, 7) * 10 + int_max(3, 7);")), 37);
    assert_eq!(check(&prog("  return int_min(0 - 2, 0 - 9) + int_max(0 - 2, 0 - 9);")), -11);
}

#[test]
fn int_pow_cases() {
    assert_eq!(check(&prog("  return int_pow(2, 10);")), 1024);
    assert_eq!(check(&prog("  return int_pow(3, 0);")), 1);
    assert_eq!(check(&prog("  return int_pow(5, 1);")), 5);
    assert_eq!(check(&prog("  return int_pow(2, 0 - 3);")), 0); // negative exponent → 0
    assert_eq!(check(&prog("  return int_pow(0 - 2, 3);")), -8);
}

#[test]
fn generic_array_push_and_repeat() {
    // array_push grows a generic array; array_repeat fills one. Element type is
    // inferred from the argument and monomorphized.
    let src = "\
fn build<T>(seed: T, a: T, b: T) -> Array<T> {
  let xs: Array<T> = array_repeat(seed, 0);
  let xs2: Array<T> = array_push(xs, a);
  return array_push(xs2, b);
}
fn main() -> Int {
  let nums: Array<Int> = build(0, 7, 9);
  let filled: Array<Int> = array_repeat(5, 3);
  return nums[0] + nums[1] + nums.len + filled[2] + filled.len;
}";
    // 7 + 9 + 2 + 5 + 3 = 26
    assert_eq!(check(src), 26);
}

#[test]
fn generic_array_managed_elements() {
    // String elements exercise the per-slot clone / consumed-seed drop paths.
    let src = "\
import std.core;
fn build<T>(seed: T, a: T) -> Array<T> {
  let xs: Array<T> = array_repeat(seed, 0);
  return array_push(xs, a);
}
fn main() -> Int {
  let words: Array<String> = build(\"x\", \"hello\");
  let filled: Array<String> = array_repeat(\"ab\", 4);
  return string_len(words[0]) + words.len + string_len(filled[3]) + filled.len;
}";
    // len("hello")=5 + 1 + len("ab")=2 + 4 = 12
    assert_eq!(check(src), 12);
}

#[test]
fn string_to_int_cases() {
    assert_eq!(check(&prog("  return string_to_int(\"42\");")), 42);
    assert_eq!(check(&prog("  return string_to_int(\"-17\");")), -17);
    assert_eq!(check(&prog("  return string_to_int(\"0\");")), 0);
    assert_eq!(check(&prog("  return string_to_int(\"\");")), 0); // empty → 0
    assert_eq!(check(&prog("  return string_to_int(\"-\");")), 0); // lone minus → 0
    assert_eq!(check(&prog("  return string_to_int(\"12x3\");")), 0); // non-digit → 0
    assert_eq!(check(&prog("  return string_to_int(\"007\");")), 7);
}

#[test]
fn string_index_of_and_contains() {
    assert_eq!(check(&prog("  return string_index_of(\"hello world\", \"wor\");")), 6);
    assert_eq!(check(&prog("  return string_index_of(\"hello\", \"xyz\");")), -1);
    assert_eq!(check(&prog("  return string_index_of(\"hello\", \"\");")), 0);
    assert_eq!(check(&prog("  return string_index_of(\"abcabc\", \"bc\");")), 1);
    assert_eq!(check(&prog("  return string_index_of(\"ab\", \"abc\");")), -1);
    let src = prog(
        "  let mut r: Int = 0;\n\
\x20 if string_contains(\"banana\", \"nan\") { r = r + 10; }\n\
\x20 if string_contains(\"banana\", \"xy\") { r = r + 1; }\n\
\x20 return r;",
    );
    assert_eq!(check(&src), 10);
}

#[test]
fn string_repeat_basic() {
    assert_eq!(check(&prog("  return string_len(string_repeat(\"ab\", 3));")), 6);
    assert_eq!(check(&prog("  return string_len(string_repeat(\"x\", 0));")), 0);
    assert_eq!(check(&prog("  return string_len(string_repeat(\"hi\", 0 - 4));")), 0);
    // content check: first byte of "abab..." is 'a' (97).
    assert_eq!(
        check(&prog("  return string_byte_at(string_repeat(\"ab\", 5), 3);")),
        98 // 'b'
    );
}

#[test]
fn string_to_upper_lower() {
    // content: 'h'→'H'(72), 'E'→'e'(101); non-letters pass through.
    assert_eq!(check(&prog("  return string_byte_at(string_to_upper(\"hello\"), 0);")), 72);
    assert_eq!(check(&prog("  return string_byte_at(string_to_lower(\"HELLO\"), 0);")), 104);
    // digit/underscore untouched by either map: '9'(57), '_'(95).
    assert_eq!(check(&prog("  return string_byte_at(string_to_upper(\"a9_\"), 1);")), 57);
    assert_eq!(check(&prog("  return string_byte_at(string_to_lower(\"A9_\"), 2);")), 95);
    // length is preserved; mixed-case round trips byte-for-byte through the oracle.
    assert_eq!(check(&prog("  return string_len(string_to_upper(\"MiXeD123\"));")), 8);
    // 'Z'(90) already upper stays; 'a'(97)→'A'(65): sum 65+90 from a 2-char map.
    assert_eq!(
        check(&prog(
            "  let u: String = string_to_upper(\"aZ\");\n  return string_byte_at(u, 0) + string_byte_at(u, 1);"
        )),
        65 + 90
    );
}

#[test]
fn string_trim_cases() {
    assert_eq!(check(&prog("  return string_len(string_trim(\"  abc  \"));")), 3);
    assert_eq!(check(&prog("  return string_len(string_trim(\"abc\"));")), 3);
    assert_eq!(check(&prog("  return string_len(string_trim(\"   \"));")), 0); // all ws
    assert_eq!(check(&prog("  return string_len(string_trim(\"\"));")), 0);
    // newlines count as whitespace (the lexer's only whitespace escape is \n).
    assert_eq!(check(&prog("  return string_len(string_trim(\"\\n x \\n\"));")), 1);
    // content survives: first byte of trimmed " hi " is 'h'(104).
    assert_eq!(check(&prog("  return string_byte_at(string_trim(\"  hi \"), 0);")), 104);
    // interior whitespace is kept — "a b" trims to itself (len 3).
    assert_eq!(check(&prog("  return string_len(string_trim(\"  a b  \"));")), 3);
}

#[test]
fn builtins_compose_in_loop() {
    // exercise refcount/drop of repeat's heap string inside a loop + search.
    let src = prog(
        "  let mut total: Int = 0;\n\
\x20 let mut i: Int = 0;\n\
\x20 while i < 20 {\n\
\x20   let s: String = string_repeat(\"ab\", i);\n\
\x20   total = total + string_len(s) + int_max(i, 5);\n\
\x20   i = i + 1;\n\
\x20 }\n\
\x20 return total;",
    );
    // sum len(2*i)=2*190=380; sum max(i,5) i=0..19 = 5*6 (i≤5) + (6+..+19=175) = 205 → 585.
    assert_eq!(check(&src), 585);
}
