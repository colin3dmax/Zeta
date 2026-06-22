// Extern (C ABI) function declarations: `extern fn name(..) -> ..;` has no body
// and is resolved by the linker. NATIVE-ONLY (the interpreter rejects extern
// calls), so these tests JIT-compile and run, asserting the result directly. The
// JIT's execution engine resolves the undefined symbols from the host process,
// so we call real libc functions (`labs`, `llabs`).
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_extern
#![cfg(feature = "llvm")]

use zeta::ast::Item;

fn run(source: &str) -> i64 {
    let structs: Vec<zeta::ast::StructDecl> = zeta::parse_source(source)
        .expect("parse")
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Struct(d) => Some(d.clone()),
            _ => None,
        })
        .collect();
    let program = zeta::lower_source(source).expect("lower");
    zeta::codegen::jit_run_i64(&program, &structs, "main").expect("native JIT")
}

#[test]
fn extern_calls_libc_labs() {
    // `labs` is C `long labs(long)` — i64 in/out, matching Zeta `Int`.
    let src = "\
extern fn labs(n: Int) -> Int;
fn main() -> Int {
  return labs(0 - 42) + labs(7);   // 42 + 7 = 49
}";
    assert_eq!(run(src), 49);
}

#[test]
fn extern_used_in_expression() {
    let src = "\
extern fn labs(n: Int) -> Int;
fn abs_diff(a: Int, b: Int) -> Int { return labs(a - b); }
fn main() -> Int {
  return abs_diff(3, 10) + abs_diff(10, 3);   // 7 + 7 = 14
}";
    assert_eq!(run(src), 14);
}

#[test]
fn extern_is_a_known_callable_with_signature() {
    // The declaration makes `llabs` a known function with a checked signature;
    // calling it type-checks and lowers like any other call.
    let src = "\
extern fn llabs(n: Int) -> Int;
fn main() -> Int {
  let x: Int = 0 - 1000000000000;   // needs 64-bit
  return llabs(x);                  // 1000000000000
}";
    assert_eq!(run(src), 1000000000000);
}
