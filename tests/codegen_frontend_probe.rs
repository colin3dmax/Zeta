// Probe: how close is the native backend to AOT-compiling the whole self-hosting
// frontend? Lowers testdata/selfhost/arena_frontend.zeta and tries to build the
// LLVM module for every function, reporting the first unsupported construct.
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_frontend_probe -- --ignored --nocapture
#![cfg(feature = "llvm")]

use zeta::ast::Item;

#[test]
#[ignore]
fn frontend_codegen_coverage() {
    let source = std::fs::read_to_string("testdata/selfhost/arena_frontend.zeta")
        .expect("frontend source should exist");
    let module = zeta::parse_source(&source).expect("should parse");
    let structs: Vec<zeta::ast::StructDecl> = module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(decl) => Some(decl.clone()),
            _ => None,
        })
        .collect();
    let program = zeta::lower_source(&source).expect("should lower");

    // Pick any existing function as the AOT entry (only its name matters here).
    let entry = program
        .functions
        .first()
        .expect("frontend has functions")
        .name
        .clone();

    let tmp = std::env::temp_dir().join("zeta_frontend_probe.o");
    let result = zeta::codegen::aot_compile_object(&program, &structs, &entry, &tmp);
    if let Err(e) = &result {
        println!(
            "\n⛔ frontend codegen stopped at: {e}\n   ({} functions, {} structs total)\n",
            program.functions.len(),
            structs.len()
        );
    }
    result.expect("the entire self-hosting frontend should lower to a native object");
    println!(
        "\n✅ ENTIRE FRONTEND ({} functions, {} structs) lowered to a native object.\n",
        program.functions.len(),
        structs.len()
    );
}
