//! Experimental native backend: MIR → LLVM IR → native code (cargo feature
//! `llvm`). Behind a feature so the default build/test needs no LLVM toolchain.
//! Targets the system LLVM 22 via inkwell `llvm22-1`.
//!
//! This module starts as a toolchain smoke test; real MIR lowering lands on top
//! once inkwell↔LLVM-22 is proven to build and JIT end-to-end.
//!
//! Build/run (arm64 macOS, system LLVM 22 from `brew install llvm`):
//!
//! ```sh
//! LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//!   cargo test --release --features llvm --lib codegen
//! ```
//!
//! The `llvm22-1-prefer-dynamic` inkwell feature links the single
//! libLLVM-22.dylib (which bundles zstd/z3/xml2), avoiding static component
//! libs whose Intel x86_64 copies under /usr/local shadow the arm64 ones.

use inkwell::context::Context;
use inkwell::OptimizationLevel;

/// JIT-compile a function `() -> i64` that returns `value`, run it, return the
/// result. Proves the inkwell ↔ LLVM 22 toolchain works end-to-end.
pub fn jit_smoke_constant(value: i64) -> i64 {
    let context = Context::create();
    let module = context.create_module("smoke");
    let builder = context.create_builder();

    let i64_type = context.i64_type();
    let fn_type = i64_type.fn_type(&[], false);
    let function = module.add_function("k", fn_type, None);
    let entry = context.append_basic_block(function, "entry");
    builder.position_at_end(entry);
    builder
        .build_return(Some(&i64_type.const_int(value as u64, true)))
        .unwrap();

    let engine = module
        .create_jit_execution_engine(OptimizationLevel::None)
        .expect("JIT engine should initialize against LLVM 22");
    unsafe {
        let compiled = engine
            .get_function::<unsafe extern "C" fn() -> i64>("k")
            .expect("smoke function should be JIT-compiled");
        compiled.call()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jit_toolchain_smoke() {
        assert_eq!(jit_smoke_constant(42), 42);
        assert_eq!(jit_smoke_constant(-7), -7);
    }
}
