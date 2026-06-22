// Native backend — generic HashSet<T> end-to-end (cargo feature `llvm`).
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_hashset
//
// Sibling of codegen_hashmap: the same trait + generics + generic-array stack,
// with the value array dropped. The JIT-compiled native result must equal the
// Stage0 interpreter's (the differential oracle).
#![cfg(feature = "llvm")]

use zeta::ast::Item;
use zeta::runtime::Value;

fn check(source: &str) -> i64 {
    let module = zeta::parse_source(source).expect("should parse");
    let structs: Vec<zeta::ast::StructDecl> = module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(decl) => Some(decl.clone()),
            _ => None,
        })
        .collect();
    let program = zeta::lower_source(source).expect("should lower");
    let oracle = match zeta::runtime::run_mir(&program).expect("interpreter should run") {
        Value::Int(n) => n,
        other => panic!("expected Int, got {other:?}"),
    };
    let native = zeta::codegen::jit_run_i64(&program, &structs, "main").expect("native JIT");
    assert_eq!(
        native, oracle,
        "native/interpreter divergence: native={native} oracle={oracle}"
    );
    oracle
}

#[test]
fn generic_hashset_string_native_matches_oracle() {
    let src = include_str!("../testdata/generic_hashset.zeta");
    // contains: alpha=1, beta=1, missing=0, e=1; size=8 distinct
    assert_eq!(check(src), 1 + 1 + 0 + 1 + 8);
}
