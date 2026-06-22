// Native backend — `import std.collections` injects the generic HashMap/HashSet
// library (cargo feature `llvm`).
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_std_collections
//
// The user program defines NO container types — `import std.collections;` pulls
// in HashMap<K,V>, HashSet<T>, and the Hash/Eq traits verbatim. Both the
// interpreter (oracle) and the native JIT compile the injected source; their
// Int results must match.
#![cfg(feature = "llvm")]

use zeta::ast::Item;
use zeta::runtime::Value;

fn check(source: &str) -> i64 {
    // `parse_source` performs prelude injection, so the struct list the codegen
    // needs (HashMap/HashSet) and the lowered program agree.
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
    assert_eq!(native, oracle, "native/interpreter divergence");
    oracle
}

#[test]
fn import_collections_hashmap_hashset() {
    let src = include_str!("../testdata/use_collections.zeta");
    assert_eq!(check(src), 10);
}

#[test]
fn import_collections_provides_struct_decls() {
    // The injected library's struct declarations are visible to the caller via
    // the normal parse — proof the prelude really merged into the user module.
    let module = zeta::parse_source(include_str!("../testdata/use_collections.zeta")).expect("parse");
    let names: Vec<&str> = module
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Struct(d) => Some(d.name.as_str()),
            _ => None,
        })
        .collect();
    assert!(names.contains(&"HashMap"), "HashMap injected, got {names:?}");
    assert!(names.contains(&"HashSet"), "HashSet injected, got {names:?}");
}
