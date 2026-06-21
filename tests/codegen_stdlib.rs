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
