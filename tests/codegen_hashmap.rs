// Native backend — generic HashMap<K, V> end-to-end (cargo feature `llvm`).
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_hashmap
//
// A generic open-addressing hash map written entirely in Zeta — the capstone of
// the trait + generics + generic-array stack: `Hash`/`Eq` traits with bounds,
// `HashMap<K, V>` generic struct, `Array<T>` fields, `array_repeat`/index-assign,
// and linear-probing resize. The JIT-compiled native result must equal the
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
fn generic_hashmap_string_int_native_matches_oracle() {
    let src = include_str!("../testdata/generic_hashmap.zeta");
    // one=1, two=22 (overwritten), three=3, missing=-1, e=50, size=8
    assert_eq!(check(src), 1 + 22 + 3 - 1 + 50 + 8);
}
