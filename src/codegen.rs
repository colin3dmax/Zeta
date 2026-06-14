//! Experimental native backend: MIR → LLVM IR → native code (cargo feature
//! `llvm`). Behind a feature so the default build/test needs no LLVM toolchain.
//! Targets the system LLVM 22 via inkwell `llvm22-1`.
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
//!
//! Supported subset: Int/Bool (i64), arithmetic/bitwise/comparison/logical, unary
//! ops, `let`/assignment, `if`/`while`/`break`/`continue`, user calls + recursion,
//! and **structs** (value semantics): struct literals, field read/write, struct
//! locals/params/returns, nesting. Strings (immutable `{len, ptr<i8>}`): literals,
//! `string_len`/`string_byte_at`/`string_byte_slice`/`string_concat`,
//! `int_to_string` (via libc snprintf), and the `ascii_is_*` predicates.
//! Enums (`{i64 tag, i64 payload}`, Int/no-payload variants) + `match` (lowered to
//! an LLVM `switch` over the tag, or over an Int/Bool value). `for` loops:
//! `for i in a..b`, `for x in intArray`, and C-style `for (init; cond; step)`.

use crate::ast::{BinaryOp, StructDecl, UnaryOp};
use crate::mir::{MirExpr, MirPattern, MirPlace, MirStmt, Program};
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::types::{BasicType, BasicTypeEnum, StructType};
use inkwell::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue};
use inkwell::{IntPredicate, OptimizationLevel};
use std::collections::HashMap;

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

/// A Zeta value type as seen by codegen. Int and Bool are both `i64`; struct
/// types carry their declared name.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ZType {
    Int,
    Struct(String),
    /// A dynamic array, represented at runtime as `{ i64 len, ptr data }` with
    /// `data` pointing at a heap (malloc) buffer of elements. Value semantics is
    /// realized by eagerly deep-copying the buffer at binding points (let /
    /// assignment / argument), so each owner has its own buffer — observably
    /// identical to the interpreter's copy-on-write. Only Int elements for now.
    Array(Box<ZType>),
    /// A string, represented at runtime as `{ i64 len, ptr<i8> data }` — the same
    /// `{len, ptr}` layout as [`ZType::Array`], but `data` points at a byte buffer
    /// and `len` is the byte count. Zeta strings are IMMUTABLE (no `s[i] = ...`),
    /// so multiple owners can share one read-only buffer — no deep copy at binding
    /// points is needed. Literals lower to a private global constant; `concat` /
    /// `byte_slice` allocate fresh malloc'd buffers.
    Str,
    /// An enum (tagged union), represented at runtime as `{ i64 tag, i64 payload }`
    /// where `tag` is the variant's declaration index and `payload` holds an Int
    /// payload (or 0 for payload-less variants). Only Int / no-payload variants are
    /// in the native subset for now (String/struct payloads would need a wider
    /// slot). The interpreter's by-name enum value is never observed directly (main
    /// returns Int), so this internal tag layout need not match it.
    Enum(String),
}

/// Per-struct layout: field name → index (declaration order) and each field's
/// type, plus the LLVM struct type. Field ORDER is internal and need not match
/// the interpreter's by-name map — `main` returns an Int, so the differential
/// oracle never observes the layout.
struct StructInfo<'ctx> {
    fields: Vec<(String, ZType)>,
    ty: StructType<'ctx>,
}

struct Types<'ctx> {
    context: &'ctx Context,
    structs: HashMap<String, StructInfo<'ctx>>,
    /// Enum name → variants in declaration order (the index is the runtime tag),
    /// each with its optional payload type.
    enums: HashMap<String, Vec<(String, Option<ZType>)>>,
    /// Function name → return type (so calls know their result type).
    returns: HashMap<String, ZType>,
}

impl<'ctx> Types<'ctx> {
    fn build(
        context: &'ctx Context,
        struct_decls: &[StructDecl],
        program: &Program,
    ) -> Result<Self, String> {
        let names: Vec<&str> = struct_decls.iter().map(|d| d.name.as_str()).collect();
        let enum_names: Vec<&str> = program.enums.iter().map(|e| e.name.as_str()).collect();
        // Pass 1: opaque named struct types (so fields can reference each other).
        let mut opaque: HashMap<String, StructType> = HashMap::new();
        for decl in struct_decls {
            opaque.insert(decl.name.clone(), context.opaque_struct_type(&decl.name));
        }
        // Pass 2: resolve field types and set bodies.
        let mut structs = HashMap::new();
        for decl in struct_decls {
            let mut fields = Vec::with_capacity(decl.fields.len());
            let mut field_llvm: Vec<BasicTypeEnum> = Vec::with_capacity(decl.fields.len());
            for field in &decl.fields {
                let zt = parse_ztype(&field.ty, &names, &enum_names)?;
                field_llvm.push(llvm_type_of(context, &zt, &opaque));
                fields.push((field.name.clone(), zt));
            }
            let ty = opaque[&decl.name];
            ty.set_body(&field_llvm, false);
            structs.insert(decl.name.clone(), StructInfo { fields, ty });
        }

        // Enum tables: variant order (= tag) + each variant's optional payload type.
        let mut enums = HashMap::new();
        for enum_decl in &program.enums {
            let mut variants = Vec::with_capacity(enum_decl.variants.len());
            for variant in &enum_decl.variants {
                let payload = match &variant.payload_type {
                    Some(t) => Some(parse_ztype(t, &names, &enum_names)?),
                    None => None,
                };
                variants.push((variant.name.clone(), payload));
            }
            enums.insert(enum_decl.name.clone(), variants);
        }

        let mut returns = HashMap::new();
        for function in &program.functions {
            let zt = match &function.return_type {
                Some(t) => parse_ztype(t, &names, &enum_names).unwrap_or(ZType::Int),
                None => ZType::Int, // Unit-returning → i64 0
            };
            returns.insert(function.name.clone(), zt);
        }

        Ok(Types {
            context,
            structs,
            enums,
            returns,
        })
    }

    fn llvm(&self, zt: &ZType) -> BasicTypeEnum<'ctx> {
        match zt {
            ZType::Int => self.context.i64_type().into(),
            ZType::Struct(name) => self.structs[name].ty.into(),
            ZType::Array(_) | ZType::Str => array_struct_type(self.context).into(),
            ZType::Enum(_) => enum_struct_type(self.context).into(),
        }
    }

    /// Resolve `enum_name.variant` to `(tag, payload_type)`; tag is the variant's
    /// declaration index.
    fn variant_tag(&self, enum_name: &str, variant: &str) -> Result<(u64, Option<ZType>), String> {
        let variants = self
            .enums
            .get(enum_name)
            .ok_or_else(|| format!("unknown enum `{enum_name}`"))?;
        variants
            .iter()
            .position(|(name, _)| name == variant)
            .map(|i| (i as u64, variants[i].1.clone()))
            .ok_or_else(|| format!("enum `{enum_name}` has no variant `{variant}`"))
    }

    fn field_index(&self, struct_name: &str, field: &str) -> Result<(u32, ZType), String> {
        let info = self
            .structs
            .get(struct_name)
            .ok_or_else(|| format!("unknown struct `{struct_name}`"))?;
        info.fields
            .iter()
            .position(|(name, _)| name == field)
            .map(|i| (i as u32, info.fields[i].1.clone()))
            .ok_or_else(|| format!("unknown field `{field}` on `{struct_name}`"))
    }
}

fn parse_ztype(text: &str, struct_names: &[&str], enum_names: &[&str]) -> Result<ZType, String> {
    match text {
        "Int" | "Bool" => Ok(ZType::Int),
        "Unit" => Ok(ZType::Int),
        "String" => Ok(ZType::Str),
        "IntArray" => Ok(ZType::Array(Box::new(ZType::Int))),
        name if struct_names.contains(&name) => Ok(ZType::Struct(name.to_string())),
        name if enum_names.contains(&name) => Ok(ZType::Enum(name.to_string())),
        other => Err(format!("type `{other}` not in the native subset")),
    }
}

/// Return type of a std builtin understood by the native backend, or `None` if
/// `callee` is not a (supported) builtin — then it must be a user function. Kept
/// in sync with [`FnLower::lower_builtin`] and `runtime::is_std_builtin`.
fn builtin_return_type(callee: &str) -> Option<ZType> {
    match callee {
        "string_len" | "string_byte_at" | "ascii_is_digit" | "ascii_is_alpha"
        | "ascii_is_alnum" | "ascii_is_whitespace" => Some(ZType::Int),
        "string_concat" | "string_byte_slice" | "int_to_string" => Some(ZType::Str),
        _ => None,
    }
}

/// The `{ i64 len, ptr data }` value type used for all arrays.
fn array_struct_type(context: &Context) -> StructType {
    context.struct_type(
        &[
            context.i64_type().into(),
            context.ptr_type(inkwell::AddressSpace::default()).into(),
        ],
        false,
    )
}

/// The `{ i64 tag, i64 payload }` value type used for all enums (E1 subset).
fn enum_struct_type(context: &Context) -> StructType {
    let i64_ty = context.i64_type();
    context.struct_type(&[i64_ty.into(), i64_ty.into()], false)
}

fn llvm_type_of<'ctx>(
    context: &'ctx Context,
    zt: &ZType,
    opaque: &HashMap<String, StructType<'ctx>>,
) -> BasicTypeEnum<'ctx> {
    match zt {
        ZType::Int => context.i64_type().into(),
        ZType::Struct(name) => opaque[name].into(),
        ZType::Array(_) | ZType::Str => array_struct_type(context).into(),
        ZType::Enum(_) => enum_struct_type(context).into(),
    }
}

/// JIT-compile `program` and run its no-arg, Int-returning `entry` to an `i64`.
/// The Stage0 interpreter (`run_mir`) is the differential oracle.
pub fn jit_run_i64(program: &Program, structs: &[StructDecl], entry: &str) -> Result<i64, String> {
    let context = Context::create();
    let types = Types::build(&context, structs, program)?;
    let module = build_module(&context, &types, program)?;
    let engine = module
        .create_jit_execution_engine(OptimizationLevel::None)
        .map_err(|e| format!("JIT engine init failed: {e}"))?;
    unsafe {
        let compiled = engine
            .get_function::<unsafe extern "C" fn() -> i64>(entry)
            .map_err(|e| format!("entry `{entry}` not found: {e}"))?;
        Ok(compiled.call())
    }
}

/// Like [`jit_run_i64`] but the entry takes one `i64` argument and the module is
/// run through LLVM `-O3` before JIT — real optimized native code. The runtime
/// `arg` keeps the optimizer from constant-folding the computation away.
pub fn jit_run_i64_arg(
    program: &Program,
    structs: &[StructDecl],
    entry: &str,
    arg: i64,
) -> Result<i64, String> {
    let context = Context::create();
    let types = Types::build(&context, structs, program)?;
    let module = build_module(&context, &types, program)?;
    optimize_module(&module)?;
    let engine = module
        .create_jit_execution_engine(OptimizationLevel::Aggressive)
        .map_err(|e| format!("JIT engine init failed: {e}"))?;
    unsafe {
        let compiled = engine
            .get_function::<unsafe extern "C" fn(i64) -> i64>(entry)
            .map_err(|e| format!("entry `{entry}` not found: {e}"))?;
        Ok(compiled.call(arg))
    }
}

/// Compile `entry` (one `i64` arg) to optimized native, then time ONLY the call.
pub fn jit_time_i64_arg(
    program: &Program,
    structs: &[StructDecl],
    entry: &str,
    arg: i64,
) -> Result<(i64, std::time::Duration), String> {
    let context = Context::create();
    let types = Types::build(&context, structs, program)?;
    let module = build_module(&context, &types, program)?;
    optimize_module(&module)?;
    let engine = module
        .create_jit_execution_engine(OptimizationLevel::Aggressive)
        .map_err(|e| format!("JIT engine init failed: {e}"))?;
    unsafe {
        let compiled = engine
            .get_function::<unsafe extern "C" fn(i64) -> i64>(entry)
            .map_err(|e| format!("entry `{entry}` not found: {e}"))?;
        let start = std::time::Instant::now();
        let result = compiled.call(arg);
        let elapsed = start.elapsed();
        Ok((result, elapsed))
    }
}

/// A long-running service whose `step` runs as **optimized native code** and can
/// be hot-swapped without losing state (native counterpart of
/// `runtime::ServiceDriver`; see docs/compiler/hot-reload-design.md). Convention:
/// `fn init() -> Int` and `reloadable fn step(state: Int, input: Int) -> Int`.
///
/// The accumulated state lives here (an `i64`); each tick calls the current
/// native `step`. `reload` JIT-compiles a new program to native and atomically
/// repoints `step` — the state is untouched, so the new (native-speed) code
/// resumes from it. This realizes the §3.1 picture: the hot path is native, only
/// the `step` boundary is an indirect call.
pub struct NativeService {
    // Leaked context keeps the JIT'd code's types alive for the engine's life.
    engine: inkwell::execution_engine::ExecutionEngine<'static>,
    step_addr: usize,
    state: i64,
}

impl NativeService {
    pub fn start(program: &Program, structs: &[StructDecl]) -> Result<NativeService, String> {
        let context: &'static Context = Box::leak(Box::new(Context::create()));
        let engine = compile_engine(context, program, structs)?;
        let init_addr = engine
            .get_function_address("init")
            .map_err(|e| format!("`init` not found: {e}"))?;
        let init: extern "C" fn() -> i64 = unsafe { std::mem::transmute(init_addr) };
        let state = init();
        let step_addr = engine
            .get_function_address("step")
            .map_err(|e| format!("`step` not found: {e}"))?;
        Ok(NativeService {
            engine,
            step_addr,
            state,
        })
    }

    /// Advance one tick by calling the current native `step(state, input)`.
    pub fn tick(&mut self, input: i64) -> i64 {
        let step: extern "C" fn(i64, i64) -> i64 = unsafe { std::mem::transmute(self.step_addr) };
        self.state = step(self.state, input);
        self.state
    }

    pub fn state(&self) -> i64 {
        self.state
    }

    /// Hot-swap to a freshly JIT-compiled native program. State is preserved.
    pub fn reload(&mut self, program: &Program, structs: &[StructDecl]) -> Result<(), String> {
        let context: &'static Context = Box::leak(Box::new(Context::create()));
        let engine = compile_engine(context, program, structs)?;
        let step_addr = engine
            .get_function_address("step")
            .map_err(|e| format!("`step` not found: {e}"))?;
        // Point at the new code BEFORE dropping the old engine (which unmaps the
        // old code); the preserved `state` i64 carries straight over.
        self.step_addr = step_addr;
        self.engine = engine;
        Ok(())
    }
}

/// The `{ i64 len, ptr data }` array value, laid out for the C ABI so it can
/// cross the Rust↔native boundary (returned/passed in two registers on arm64).
/// The `data` buffer is libc-malloc'd by the JIT'd code, so it lives on the C
/// heap and SURVIVES an engine swap — only the code is unmapped on reload.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct NativeArray {
    pub len: i64,
    pub data: *mut i64,
}

/// Like [`NativeService`] but the threaded state is an `IntArray`, proving native
/// hot-reload works with a non-scalar, heap-backed state. Convention:
/// `fn init() -> IntArray` and `reloadable fn step(state: IntArray, input: Int) -> IntArray`.
pub struct NativeArrayService {
    engine: inkwell::execution_engine::ExecutionEngine<'static>,
    step_addr: usize,
    state: NativeArray,
}

impl NativeArrayService {
    pub fn start(program: &Program, structs: &[StructDecl]) -> Result<NativeArrayService, String> {
        let context: &'static Context = Box::leak(Box::new(Context::create()));
        let engine = compile_engine(context, program, structs)?;
        let init_addr = engine
            .get_function_address("init")
            .map_err(|e| format!("`init` not found: {e}"))?;
        let init: extern "C" fn() -> NativeArray = unsafe { std::mem::transmute(init_addr) };
        let state = init();
        let step_addr = engine
            .get_function_address("step")
            .map_err(|e| format!("`step` not found: {e}"))?;
        Ok(NativeArrayService {
            engine,
            step_addr,
            state,
        })
    }

    pub fn tick(&mut self, input: i64) {
        let step: extern "C" fn(NativeArray, i64) -> NativeArray =
            unsafe { std::mem::transmute(self.step_addr) };
        self.state = step(self.state, input);
    }

    pub fn len(&self) -> i64 {
        self.state.len
    }

    /// Read element `i` of the current state buffer.
    pub fn get(&self, i: i64) -> i64 {
        assert!(i >= 0 && i < self.state.len, "index out of bounds");
        unsafe { *self.state.data.offset(i as isize) }
    }

    pub fn reload(&mut self, program: &Program, structs: &[StructDecl]) -> Result<(), String> {
        let context: &'static Context = Box::leak(Box::new(Context::create()));
        let engine = compile_engine(context, program, structs)?;
        let step_addr = engine
            .get_function_address("step")
            .map_err(|e| format!("`step` not found: {e}"))?;
        self.step_addr = step_addr;
        self.engine = engine;
        Ok(())
    }
}

/// Build + optimize a module and wrap it in an aggressive JIT engine.
fn compile_engine(
    context: &'static Context,
    program: &Program,
    structs: &[StructDecl],
) -> Result<inkwell::execution_engine::ExecutionEngine<'static>, String> {
    let types = Types::build(context, structs, program)?;
    let module = build_module(context, &types, program)?;
    optimize_module(&module)?;
    module
        .create_jit_execution_engine(OptimizationLevel::Aggressive)
        .map_err(|e| format!("JIT engine init failed: {e}"))
}

fn build_module<'ctx>(
    context: &'ctx Context,
    types: &Types<'ctx>,
    program: &Program,
) -> Result<inkwell::module::Module<'ctx>, String> {
    let module = context.create_module("zeta_native");
    let builder = context.create_builder();
    let struct_names: Vec<&str> = types.structs.keys().map(|s| s.as_str()).collect();
    let enum_names: Vec<&str> = types.enums.keys().map(|s| s.as_str()).collect();

    // libc malloc/memcpy for array buffers + deep copies (link via libc).
    let ptr_ty = context.ptr_type(inkwell::AddressSpace::default());
    let i64_ty = context.i64_type();
    let malloc = module.add_function("malloc", ptr_ty.fn_type(&[i64_ty.into()], false), None);
    let memcpy = module.add_function(
        "memcpy",
        ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64_ty.into()], false),
        None,
    );
    // libc snprintf for int_to_string (variadic: i32 snprintf(ptr, size_t, fmt, ...)).
    let snprintf = module.add_function(
        "snprintf",
        context
            .i32_type()
            .fn_type(&[ptr_ty.into(), i64_ty.into(), ptr_ty.into()], true),
        None,
    );

    // Pass 1: declare every function with its typed signature.
    let mut functions: HashMap<String, FunctionValue> = HashMap::new();
    for function in &program.functions {
        let mut param_types = Vec::with_capacity(function.params.len());
        for param in &function.params {
            let zt = parse_ztype(&param.ty, &struct_names, &enum_names)?;
            param_types.push(types.llvm(&zt).into());
        }
        let ret = &types.returns[&function.name];
        let fn_type = types.llvm(ret).fn_type(&param_types, false);
        functions.insert(
            function.name.clone(),
            module.add_function(&function.name, fn_type, None),
        );
    }

    // Pass 2: lower each body.
    for function in &program.functions {
        let llvm_fn = functions[&function.name];
        let entry_bb = context.append_basic_block(llvm_fn, "entry");
        builder.position_at_end(entry_bb);

        // Infer the type of every local so we can pre-allocate typed slots.
        let mut env: HashMap<String, ZType> = HashMap::new();
        for param in &function.params {
            env.insert(
                param.name.clone(),
                parse_ztype(&param.ty, &struct_names, &enum_names)?,
            );
        }
        infer_locals(&function.body, types, &struct_names, &enum_names, &mut env)?;

        let mut lower = FnLower {
            context,
            builder: &builder,
            types,
            functions: &functions,
            malloc,
            memcpy,
            snprintf,
            llvm_fn,
            entry_bb,
            locals: HashMap::new(),
            loops: Vec::new(),
        };
        for (name, zt) in &env {
            let slot = lower.entry_alloca(name, types.llvm(zt));
            lower.locals.insert(name.clone(), (slot, zt.clone()));
        }
        for (index, param) in function.params.iter().enumerate() {
            let value = llvm_fn.get_nth_param(index as u32).expect("param exists");
            builder.build_store(lower.locals[&param.name].0, value).unwrap();
        }

        let terminated = lower.lower_stmts(&function.body)?;
        if !terminated {
            let ret = &types.returns[&function.name];
            let zero = lower.zero_of(ret);
            builder.build_return(Some(&zero)).unwrap();
        }
    }

    module
        .verify()
        .map_err(|e| format!("LLVM module verification failed: {e}"))?;
    Ok(module)
}

/// Lightweight type inference: record the ZType bound to every `let` (and used
/// in nested blocks), so codegen can pre-allocate correctly typed slots.
fn infer_locals(
    stmts: &[MirStmt],
    types: &Types,
    struct_names: &[&str],
    enum_names: &[&str],
    env: &mut HashMap<String, ZType>,
) -> Result<(), String> {
    for stmt in stmts {
        match stmt {
            MirStmt::Local {
                name, ty, value, ..
            } => {
                let zt = match ty {
                    Some(t) => parse_ztype(t, struct_names, enum_names)?,
                    None => infer_expr_type(value, types, env)?,
                };
                env.insert(name.clone(), zt);
            }
            MirStmt::If {
                then_body,
                else_body,
                ..
            } => {
                infer_locals(then_body, types, struct_names, enum_names, env)?;
                infer_locals(else_body, types, struct_names, enum_names, env)?;
            }
            MirStmt::While { body, .. } => {
                infer_locals(body, types, struct_names, enum_names, env)?
            }
            MirStmt::ForRange { binding, body, .. } => {
                env.insert(binding.clone(), ZType::Int);
                infer_locals(body, types, struct_names, enum_names, env)?;
            }
            MirStmt::ForIn {
                binding,
                iterable,
                body,
            } => {
                let elem = match infer_expr_type(iterable, types, env)? {
                    ZType::Array(elem) => *elem,
                    _ => return Err("for-in iterable must be an array".into()),
                };
                env.insert(binding.clone(), elem);
                infer_locals(body, types, struct_names, enum_names, env)?;
            }
            MirStmt::ForC {
                init, step, body, ..
            } => {
                infer_locals(std::slice::from_ref(init), types, struct_names, enum_names, env)?;
                infer_locals(std::slice::from_ref(step), types, struct_names, enum_names, env)?;
                infer_locals(body, types, struct_names, enum_names, env)?;
            }
            MirStmt::Match { value, arms } => {
                // Each arm's pattern may bind a local; register its type, then
                // recurse into the arm body (whose `let`s also need slots).
                let value_ty = infer_expr_type(value, types, env)?;
                for arm in arms {
                    match &arm.pattern {
                        MirPattern::Name(name) => {
                            env.insert(name.clone(), value_ty.clone());
                        }
                        MirPattern::Variant {
                            enum_name, binding, ..
                        } => {
                            if let Some(binding) = binding {
                                // Payload-binding type comes from the variant decl.
                                let payload = match &value_ty {
                                    ZType::Enum(_) => arm_variant_payload(types, enum_name, &arm.pattern),
                                    _ => None,
                                };
                                env.insert(binding.clone(), payload.unwrap_or(ZType::Int));
                            }
                        }
                        _ => {}
                    }
                    infer_locals(&arm.body, types, struct_names, enum_names, env)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Payload type bound by a `Variant` arm pattern, if any.
fn arm_variant_payload(types: &Types, enum_name: &str, pattern: &MirPattern) -> Option<ZType> {
    if let MirPattern::Variant { variant, .. } = pattern {
        types
            .enums
            .get(enum_name)
            .and_then(|variants| variants.iter().find(|(n, _)| n == variant))
            .and_then(|(_, payload)| payload.clone())
    } else {
        None
    }
}

fn infer_expr_type(
    expr: &MirExpr,
    types: &Types,
    env: &HashMap<String, ZType>,
) -> Result<ZType, String> {
    Ok(match expr {
        MirExpr::Int(_) | MirExpr::Bool(_) | MirExpr::Binary { .. } | MirExpr::Unary { .. } => {
            ZType::Int
        }
        MirExpr::Load(name) => env
            .get(name)
            .cloned()
            .ok_or_else(|| format!("type of unknown local `{name}`"))?,
        MirExpr::String(_) => ZType::Str,
        MirExpr::EnumVariant { enum_name, .. } => ZType::Enum(enum_name.clone()),
        MirExpr::StructLiteral { ty, .. } => ZType::Struct(ty.clone()),
        MirExpr::ArrayLiteral { .. } => ZType::Array(Box::new(ZType::Int)),
        MirExpr::Index { base, .. } => match infer_expr_type(base, types, env)? {
            ZType::Array(elem) => *elem,
            _ => return Err("index of non-array".into()),
        },
        MirExpr::FieldAccess { base, field } => {
            let base_ty = infer_expr_type(base, types, env)?;
            match base_ty {
                ZType::Struct(name) => types.field_index(&name, field)?.1,
                ZType::Array(_) if field == "len" => ZType::Int,
                _ => return Err("field access on non-struct".into()),
            }
        }
        MirExpr::Call { callee, .. } => match builtin_return_type(callee) {
            Some(zt) => zt,
            None => types
                .returns
                .get(callee)
                .cloned()
                .ok_or_else(|| format!("unknown function `{callee}`"))?,
        },
    })
}

fn host_target_machine() -> Result<inkwell::targets::TargetMachine, String> {
    use inkwell::targets::{CodeModel, InitializationConfig, RelocMode, Target, TargetMachine};

    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| format!("native target init failed: {e}"))?;
    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple).map_err(|e| format!("target lookup failed: {e}"))?;
    target
        .create_target_machine(
            &triple,
            TargetMachine::get_host_cpu_name().to_str().unwrap_or(""),
            TargetMachine::get_host_cpu_features().to_str().unwrap_or(""),
            OptimizationLevel::Aggressive,
            RelocMode::PIC,
            CodeModel::Default,
        )
        .ok_or_else(|| "could not create host target machine".to_string())
}

fn optimize_module(module: &inkwell::module::Module) -> Result<(), String> {
    let machine = host_target_machine()?;
    module
        .run_passes(
            "default<O3>",
            &machine,
            inkwell::passes::PassBuilderOptions::create(),
        )
        .map_err(|e| format!("optimization passes failed: {e}"))
}

/// Ahead-of-time: compile `program` to a native **object file** at `path`. The
/// `entry` function (e.g. `main`) is renamed to `zeta_entry` so it won't clash
/// with the C `main` of the driver that links against this object. This is the
/// JIT-free path — `cc obj.o driver.c -o exe` yields a standalone binary, a step
/// toward dropping Stage0.
pub fn aot_compile_object(
    program: &Program,
    structs: &[StructDecl],
    entry: &str,
    path: &std::path::Path,
) -> Result<(), String> {
    let context = Context::create();
    let types = Types::build(&context, structs, program)?;
    let module = build_module(&context, &types, program)?;
    let entry_fn = module
        .get_function(entry)
        .ok_or_else(|| format!("entry `{entry}` not found"))?;
    entry_fn.as_global_value().set_name("zeta_entry");
    optimize_module(&module)?;
    let machine = host_target_machine()?;
    machine
        .write_to_file(&module, inkwell::targets::FileType::Object, path)
        .map_err(|e| format!("object emission failed: {e}"))
}

struct FnLower<'a, 'ctx> {
    context: &'ctx Context,
    builder: &'a Builder<'ctx>,
    types: &'a Types<'ctx>,
    functions: &'a HashMap<String, FunctionValue<'ctx>>,
    malloc: FunctionValue<'ctx>,
    memcpy: FunctionValue<'ctx>,
    snprintf: FunctionValue<'ctx>,
    llvm_fn: FunctionValue<'ctx>,
    entry_bb: BasicBlock<'ctx>,
    /// local name → (alloca slot, type)
    locals: HashMap<String, (PointerValue<'ctx>, ZType)>,
    /// Enclosing loops as `(continue_target, exit)`. `break` jumps to `exit`;
    /// `continue` jumps to `continue_target` — which is the condition head for
    /// `while`, but the increment/step block for `for` loops (so `continue` still
    /// advances the counter / runs the step, matching the interpreter).
    loops: Vec<(BasicBlock<'ctx>, BasicBlock<'ctx>)>,
}

impl<'a, 'ctx> FnLower<'a, 'ctx> {
    fn i64t(&self) -> inkwell::types::IntType<'ctx> {
        self.context.i64_type()
    }

    fn zero_of(&self, zt: &ZType) -> BasicValueEnum<'ctx> {
        match zt {
            ZType::Int => self.i64t().const_zero().into(),
            ZType::Struct(name) => self.types.structs[name].ty.const_zero().into(),
            ZType::Array(_) | ZType::Str => array_struct_type(self.context).const_zero().into(),
            ZType::Enum(_) => enum_struct_type(self.context).const_zero().into(),
        }
    }

    /// Apply value-semantics at a binding point: if `value` is an array, return a
    /// deep copy (fresh malloc'd buffer) so the new owner is independent; other
    /// types are already value types in LLVM and pass through.
    fn bind_value(&self, value: BasicValueEnum<'ctx>, zt: &ZType) -> BasicValueEnum<'ctx> {
        if matches!(zt, ZType::Array(_)) {
            self.deep_copy_array(value.into_struct_value()).into()
        } else {
            value
        }
    }

    /// Deep-copy an `{len, data}` array value: malloc a new buffer, memcpy the
    /// elements, return `{len, newdata}`.
    fn deep_copy_array(
        &self,
        arr: inkwell::values::StructValue<'ctx>,
    ) -> inkwell::values::StructValue<'ctx> {
        let b = self.builder;
        let len = b.build_extract_value(arr, 0, "len").unwrap().into_int_value();
        let src = b.build_extract_value(arr, 1, "data").unwrap().into_pointer_value();
        let bytes = b
            .build_int_mul(len, self.i64t().const_int(8, false), "bytes")
            .unwrap();
        let dst = b
            .build_call(self.malloc, &[bytes.into()], "buf")
            .unwrap()
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value();
        b.build_call(self.memcpy, &[dst.into(), src.into(), bytes.into()], "cp")
            .unwrap();
        let with_len = b
            .build_insert_value(array_struct_type(self.context).get_undef(), len, 0, "a0")
            .unwrap();
        b.build_insert_value(with_len, dst, 1, "a1")
            .unwrap()
            .into_struct_value()
    }

    /// Allocate a slot of `ty` at the TOP of the entry block (mem2reg-friendly).
    fn entry_alloca(&self, name: &str, ty: BasicTypeEnum<'ctx>) -> PointerValue<'ctx> {
        let saved = self.builder.get_insert_block();
        match self.entry_bb.get_first_instruction() {
            Some(first) => self.builder.position_before(&first),
            None => self.builder.position_at_end(self.entry_bb),
        }
        let slot = self.builder.build_alloca(ty, name).unwrap();
        if let Some(block) = saved {
            self.builder.position_at_end(block);
        }
        slot
    }

    fn lower_stmts(&mut self, stmts: &[MirStmt]) -> Result<bool, String> {
        for stmt in stmts {
            if self.lower_stmt(stmt)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn lower_stmt(&mut self, stmt: &MirStmt) -> Result<bool, String> {
        match stmt {
            MirStmt::Local { name, value, .. } => {
                let (v, vt) = self.lower_expr(value)?;
                let v = self.bind_value(v, &vt);
                self.builder.build_store(self.locals[name].0, v).unwrap();
                Ok(false)
            }
            MirStmt::Store { place, value } => {
                let (v, vt) = self.lower_expr(value)?;
                let v = self.bind_value(v, &vt);
                let (slot, _) = self.resolve_place(place)?;
                self.builder.build_store(slot, v).unwrap();
                Ok(false)
            }
            MirStmt::Return(value) => {
                match value {
                    Some(expr) => {
                        let (v, _) = self.lower_expr(expr)?;
                        self.builder.build_return(Some(&v)).unwrap();
                    }
                    None => {
                        self.builder
                            .build_return(Some(&self.i64t().const_zero()))
                            .unwrap();
                    }
                }
                Ok(true)
            }
            MirStmt::If {
                condition,
                then_body,
                else_body,
            } => {
                let cond = self.lower_cond(condition)?;
                let then_bb = self.context.append_basic_block(self.llvm_fn, "then");
                let else_bb = self.context.append_basic_block(self.llvm_fn, "else");
                let cont_bb = self.context.append_basic_block(self.llvm_fn, "endif");
                self.builder
                    .build_conditional_branch(cond, then_bb, else_bb)
                    .unwrap();
                self.builder.position_at_end(then_bb);
                if !self.lower_stmts(then_body)? {
                    self.builder.build_unconditional_branch(cont_bb).unwrap();
                }
                self.builder.position_at_end(else_bb);
                if !self.lower_stmts(else_body)? {
                    self.builder.build_unconditional_branch(cont_bb).unwrap();
                }
                self.builder.position_at_end(cont_bb);
                Ok(false)
            }
            MirStmt::While { condition, body } => {
                let head = self.context.append_basic_block(self.llvm_fn, "while.head");
                let body_bb = self.context.append_basic_block(self.llvm_fn, "while.body");
                let exit = self.context.append_basic_block(self.llvm_fn, "while.exit");
                self.builder.build_unconditional_branch(head).unwrap();
                self.builder.position_at_end(head);
                let cond = self.lower_cond(condition)?;
                self.builder
                    .build_conditional_branch(cond, body_bb, exit)
                    .unwrap();
                self.loops.push((head, exit));
                self.builder.position_at_end(body_bb);
                if !self.lower_stmts(body)? {
                    self.builder.build_unconditional_branch(head).unwrap();
                }
                self.loops.pop();
                self.builder.position_at_end(exit);
                Ok(false)
            }
            MirStmt::Break => {
                let (_, exit) = *self.loops.last().ok_or("`break` outside loop")?;
                self.builder.build_unconditional_branch(exit).unwrap();
                Ok(true)
            }
            MirStmt::Continue => {
                let (head, _) = *self.loops.last().ok_or("`continue` outside loop")?;
                self.builder.build_unconditional_branch(head).unwrap();
                Ok(true)
            }
            MirStmt::Drop(expr) => {
                self.lower_expr(expr)?;
                Ok(false)
            }
            MirStmt::Match { value, arms } => self.lower_match(value, arms),
            MirStmt::ForRange {
                binding,
                start,
                end,
                body,
            } => self.lower_for_range(binding, start, end, body),
            MirStmt::ForIn {
                binding,
                iterable,
                body,
            } => self.lower_for_in(binding, iterable, body),
            MirStmt::ForC {
                init,
                condition,
                step,
                body,
            } => self.lower_for_c(init, condition, step, body),
        }
    }

    /// `for i in start..end`: evaluate both bounds once, then `while i < end` with
    /// `i` incremented in the latch (so `continue` still advances). Exclusive end,
    /// matching the interpreter.
    fn lower_for_range(
        &mut self,
        binding: &str,
        start: &MirExpr,
        end: &MirExpr,
        body: &[MirStmt],
    ) -> Result<bool, String> {
        let start_v = self.lower_int(start)?;
        let end_v = self.lower_int(end)?;
        let slot = self.locals[binding].0;
        self.builder.build_store(slot, start_v).unwrap();

        let head = self.context.append_basic_block(self.llvm_fn, "for.head");
        let body_bb = self.context.append_basic_block(self.llvm_fn, "for.body");
        let latch = self.context.append_basic_block(self.llvm_fn, "for.latch");
        let exit = self.context.append_basic_block(self.llvm_fn, "for.exit");

        self.builder.build_unconditional_branch(head).unwrap();
        self.builder.position_at_end(head);
        let i = self.builder.build_load(self.i64t(), slot, "i").unwrap().into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::SLT, i, end_v, "for.cmp")
            .unwrap();
        self.builder.build_conditional_branch(cond, body_bb, exit).unwrap();

        self.loops.push((latch, exit));
        self.builder.position_at_end(body_bb);
        if !self.lower_stmts(body)? {
            self.builder.build_unconditional_branch(latch).unwrap();
        }
        self.loops.pop();

        self.builder.position_at_end(latch);
        let i = self.builder.build_load(self.i64t(), slot, "i").unwrap().into_int_value();
        let next = self.builder.build_int_add(i, self.i64t().const_int(1, false), "inc").unwrap();
        self.builder.build_store(slot, next).unwrap();
        self.builder.build_unconditional_branch(head).unwrap();

        self.builder.position_at_end(exit);
        Ok(false)
    }

    /// `for x in array`: walk indices `0..len`, binding each element. Only IntArray
    /// (Int elements) is in the native subset.
    fn lower_for_in(
        &mut self,
        binding: &str,
        iterable: &MirExpr,
        body: &[MirStmt],
    ) -> Result<bool, String> {
        let (arr, arr_ty) = self.lower_expr(iterable)?;
        let ZType::Array(elem) = arr_ty else {
            return Err("for-in iterable must be an array".into());
        };
        if *elem != ZType::Int {
            return Err("for-in only supports Int-element arrays in the native subset".into());
        }
        let arr = arr.into_struct_value();
        let len = self.builder.build_extract_value(arr, 0, "len").unwrap().into_int_value();
        let data = self.builder.build_extract_value(arr, 1, "data").unwrap().into_pointer_value();

        let idx_slot = self.entry_alloca("for.idx", self.i64t().into());
        self.builder.build_store(idx_slot, self.i64t().const_zero()).unwrap();
        let binding_slot = self.locals[binding].0;

        let head = self.context.append_basic_block(self.llvm_fn, "forin.head");
        let body_bb = self.context.append_basic_block(self.llvm_fn, "forin.body");
        let latch = self.context.append_basic_block(self.llvm_fn, "forin.latch");
        let exit = self.context.append_basic_block(self.llvm_fn, "forin.exit");

        self.builder.build_unconditional_branch(head).unwrap();
        self.builder.position_at_end(head);
        let idx = self.builder.build_load(self.i64t(), idx_slot, "idx").unwrap().into_int_value();
        let cond = self
            .builder
            .build_int_compare(IntPredicate::SLT, idx, len, "forin.cmp")
            .unwrap();
        self.builder.build_conditional_branch(cond, body_bb, exit).unwrap();

        self.loops.push((latch, exit));
        self.builder.position_at_end(body_bb);
        // Bind the current element, then lower the body.
        let elem_ptr = unsafe { self.builder.build_in_bounds_gep(self.i64t(), data, &[idx], "ep").unwrap() };
        let elem_val = self.builder.build_load(self.i64t(), elem_ptr, "elem").unwrap();
        self.builder.build_store(binding_slot, elem_val).unwrap();
        if !self.lower_stmts(body)? {
            self.builder.build_unconditional_branch(latch).unwrap();
        }
        self.loops.pop();

        self.builder.position_at_end(latch);
        let idx = self.builder.build_load(self.i64t(), idx_slot, "idx").unwrap().into_int_value();
        let next = self.builder.build_int_add(idx, self.i64t().const_int(1, false), "inc").unwrap();
        self.builder.build_store(idx_slot, next).unwrap();
        self.builder.build_unconditional_branch(head).unwrap();

        self.builder.position_at_end(exit);
        Ok(false)
    }

    /// `for (init; cond; step) { body }`: init once, then `loop { if !cond break;
    /// body; step }`. `continue` jumps to the step block, matching the interpreter.
    fn lower_for_c(
        &mut self,
        init: &MirStmt,
        condition: &MirExpr,
        step: &MirStmt,
        body: &[MirStmt],
    ) -> Result<bool, String> {
        self.lower_stmt(init)?;

        let head = self.context.append_basic_block(self.llvm_fn, "forc.head");
        let body_bb = self.context.append_basic_block(self.llvm_fn, "forc.body");
        let step_bb = self.context.append_basic_block(self.llvm_fn, "forc.step");
        let exit = self.context.append_basic_block(self.llvm_fn, "forc.exit");

        self.builder.build_unconditional_branch(head).unwrap();
        self.builder.position_at_end(head);
        let cond = self.lower_cond(condition)?;
        self.builder.build_conditional_branch(cond, body_bb, exit).unwrap();

        self.loops.push((step_bb, exit));
        self.builder.position_at_end(body_bb);
        if !self.lower_stmts(body)? {
            self.builder.build_unconditional_branch(step_bb).unwrap();
        }
        self.loops.pop();

        self.builder.position_at_end(step_bb);
        self.lower_stmt(step)?;
        self.builder.build_unconditional_branch(head).unwrap();

        self.builder.position_at_end(exit);
        Ok(false)
    }

    /// Lower a `match`: switch on a single i64 scrutinee (an enum's tag, or an
    /// Int/Bool value itself). Each concrete pattern becomes a switch case; a
    /// `Name`/`Wildcard` arm is the default. Exhaustiveness is guaranteed by the
    /// MIR verifier, so when there is no catch-all the switch default is
    /// `unreachable`. Returns whether control is guaranteed terminated afterwards.
    fn lower_match(&mut self, value: &MirExpr, arms: &[crate::mir::MirMatchArm]) -> Result<bool, String> {
        let (val, vty) = self.lower_expr(value)?;
        let scrutinee = match &vty {
            ZType::Enum(_) => self
                .builder
                .build_extract_value(val.into_struct_value(), 0, "tag")
                .unwrap()
                .into_int_value(),
            ZType::Int => val.into_int_value(),
            _ => return Err("match scrutinee must be an enum or Int/Bool".into()),
        };
        // The block holding the scrutinee; the switch terminates it. Building the
        // `unreachable` default below repositions the builder, so capture it now.
        let head_bb = self.builder.get_insert_block().unwrap();

        let arm_blocks: Vec<BasicBlock<'ctx>> = arms
            .iter()
            .map(|_| self.context.append_basic_block(self.llvm_fn, "arm"))
            .collect();
        let end_bb = self.context.append_basic_block(self.llvm_fn, "match.end");

        // Map each concrete pattern to (case const, its arm block); find the
        // catch-all arm (first Name/Wildcard) to use as the switch default.
        let mut cases: Vec<(IntValue<'ctx>, BasicBlock<'ctx>)> = Vec::new();
        let mut default_bb: Option<BasicBlock<'ctx>> = None;
        for (i, arm) in arms.iter().enumerate() {
            match &arm.pattern {
                MirPattern::Name(_) | MirPattern::Wildcard => {
                    if default_bb.is_none() {
                        default_bb = Some(arm_blocks[i]);
                    }
                }
                MirPattern::Variant { enum_name, variant, .. } => {
                    let (tag, _) = self.types.variant_tag(enum_name, variant)?;
                    cases.push((self.i64t().const_int(tag, false), arm_blocks[i]));
                }
                MirPattern::Int(text) => {
                    let n: i64 = text.parse().map_err(|_| format!("bad Int pattern `{text}`"))?;
                    cases.push((self.i64t().const_int(n as u64, true), arm_blocks[i]));
                }
                MirPattern::Bool(b) => {
                    cases.push((self.i64t().const_int(*b as u64, false), arm_blocks[i]));
                }
                MirPattern::String(_) => {
                    return Err("string match patterns not in the native subset".into())
                }
            }
        }

        // Exhaustive-but-no-catch-all → an `unreachable` default block.
        let default = match default_bb {
            Some(bb) => bb,
            None => {
                let bb = self.context.append_basic_block(self.llvm_fn, "match.unreachable");
                self.builder.position_at_end(bb);
                self.builder.build_unreachable().unwrap();
                bb
            }
        };
        self.builder.position_at_end(head_bb);
        self.builder.build_switch(scrutinee, default, &cases).unwrap();

        // Lower each arm: bind its pattern variable, then its body.
        for (i, arm) in arms.iter().enumerate() {
            self.builder.position_at_end(arm_blocks[i]);
            match &arm.pattern {
                MirPattern::Name(name) => {
                    self.builder.build_store(self.locals[name].0, val).unwrap();
                }
                MirPattern::Variant {
                    enum_name,
                    variant,
                    binding: Some(binding),
                } => {
                    let (_, payload_ty) = self.types.variant_tag(enum_name, variant)?;
                    if !matches!(payload_ty, Some(ZType::Int)) {
                        return Err("only Int enum payloads are in the native subset".into());
                    }
                    let payload = self
                        .builder
                        .build_extract_value(val.into_struct_value(), 1, "payload")
                        .unwrap();
                    self.builder.build_store(self.locals[binding].0, payload).unwrap();
                }
                _ => {}
            }
            if !self.lower_stmts(&arm.body)? {
                self.builder.build_unconditional_branch(end_bb).unwrap();
            }
        }

        self.builder.position_at_end(end_bb);
        Ok(false)
    }

    /// Resolve an assignment place to (pointer-to-slot, type).
    fn resolve_place(&mut self, place: &MirPlace) -> Result<(PointerValue<'ctx>, ZType), String> {
        match place {
            MirPlace::Local(name) => {
                let (slot, zt) = self
                    .locals
                    .get(name)
                    .ok_or_else(|| format!("store to unknown local `{name}`"))?;
                Ok((*slot, zt.clone()))
            }
            MirPlace::Field { base, field } => {
                let (base_ptr, base_ty) = self.resolve_place(base)?;
                let ZType::Struct(struct_name) = base_ty else {
                    return Err("field assignment on non-struct".into());
                };
                let (index, field_ty) = self.types.field_index(&struct_name, field)?;
                let struct_ty = self.types.structs[&struct_name].ty;
                let field_ptr = self
                    .builder
                    .build_struct_gep(struct_ty, base_ptr, index, "fieldptr")
                    .map_err(|_| "struct GEP failed".to_string())?;
                Ok((field_ptr, field_ty))
            }
            MirPlace::Index { base, index } => {
                let (base_slot, base_ty) = self.resolve_place(base)?;
                let ZType::Array(elem) = base_ty else {
                    return Err("index assignment on non-array".into());
                };
                // Load the {len, data} struct from the base slot, GEP into the
                // (exclusively owned) heap buffer, and return the element ptr.
                let arr = self
                    .builder
                    .build_load(array_struct_type(self.context), base_slot, "arr")
                    .unwrap()
                    .into_struct_value();
                let data = self
                    .builder
                    .build_extract_value(arr, 1, "data")
                    .unwrap()
                    .into_pointer_value();
                let idx = self.lower_int(index)?;
                let elem_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(self.i64t(), data, &[idx], "elemptr")
                        .unwrap()
                };
                Ok((elem_ptr, *elem))
            }
        }
    }

    /// Lower an expression to (value, type).
    fn lower_expr(&mut self, expr: &MirExpr) -> Result<(BasicValueEnum<'ctx>, ZType), String> {
        match expr {
            MirExpr::Int(text) => {
                let n: i64 = text.parse().map_err(|_| format!("bad Int `{text}`"))?;
                Ok((self.i64t().const_int(n as u64, true).into(), ZType::Int))
            }
            MirExpr::Bool(b) => Ok((self.i64t().const_int(*b as u64, false).into(), ZType::Int)),
            MirExpr::Load(name) => {
                let (slot, zt) = self
                    .locals
                    .get(name)
                    .ok_or_else(|| format!("load of unknown local `{name}`"))?;
                let llvm_ty = self.types.llvm(zt);
                let value = self.builder.build_load(llvm_ty, *slot, name).unwrap();
                Ok((value, zt.clone()))
            }
            MirExpr::Unary { op, expr } => {
                let v = self.lower_int(expr)?;
                let r = match op {
                    UnaryOp::Neg => self.builder.build_int_neg(v, "neg").unwrap(),
                    UnaryOp::BitNot => self.builder.build_not(v, "bitnot").unwrap(),
                    UnaryOp::Not => {
                        let z = self
                            .builder
                            .build_int_compare(IntPredicate::EQ, v, self.i64t().const_zero(), "isz")
                            .unwrap();
                        self.builder.build_int_z_extend(z, self.i64t(), "not").unwrap()
                    }
                };
                Ok((r.into(), ZType::Int))
            }
            MirExpr::Binary { op, left, right } => {
                Ok((self.lower_binary(*op, left, right)?.into(), ZType::Int))
            }
            MirExpr::Call { callee, args } => {
                if let Some(result) = self.lower_builtin(callee, args)? {
                    return Ok(result);
                }
                let function = *self
                    .functions
                    .get(callee)
                    .ok_or_else(|| format!("call to unknown `{callee}`"))?;
                let mut argv = Vec::with_capacity(args.len());
                for arg in args {
                    let (v, vt) = self.lower_expr(arg)?;
                    argv.push(self.bind_value(v, &vt).into());
                }
                let call = self.builder.build_call(function, &argv, "call").unwrap();
                let ret = self
                    .types
                    .returns
                    .get(callee)
                    .cloned()
                    .unwrap_or(ZType::Int);
                let value = call
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| format!("`{callee}` returned no value"))?;
                Ok((value, ret))
            }
            MirExpr::StructLiteral { ty, fields } => {
                let info = self
                    .types
                    .structs
                    .get(ty)
                    .ok_or_else(|| format!("unknown struct `{ty}`"))?;
                let struct_ty = info.ty;
                // Lower field values in declaration order.
                let mut current = struct_ty.get_undef();
                let field_order: Vec<(usize, String)> = info
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(i, (n, _))| (i, n.clone()))
                    .collect();
                for (index, field_name) in field_order {
                    let value_expr = &fields
                        .iter()
                        .find(|f| f.name == field_name)
                        .ok_or_else(|| format!("missing field `{field_name}` in `{ty}` literal"))?
                        .value;
                    let (v, _) = self.lower_expr(value_expr)?;
                    current = self
                        .builder
                        .build_insert_value(current, v, index as u32, "ins")
                        .unwrap()
                        .into_struct_value();
                }
                Ok((current.into(), ZType::Struct(ty.clone())))
            }
            MirExpr::FieldAccess { base, field } => {
                let (base_val, base_ty) = self.lower_expr(base)?;
                match base_ty {
                    ZType::Struct(struct_name) => {
                        let (index, field_ty) = self.types.field_index(&struct_name, field)?;
                        let value = self
                            .builder
                            .build_extract_value(base_val.into_struct_value(), index, "field")
                            .unwrap();
                        Ok((value, field_ty))
                    }
                    ZType::Array(_) if field == "len" => {
                        let len = self
                            .builder
                            .build_extract_value(base_val.into_struct_value(), 0, "len")
                            .unwrap();
                        Ok((len, ZType::Int))
                    }
                    _ => Err(format!("field `{field}` access not in the native subset")),
                }
            }
            MirExpr::ArrayLiteral { elements } => {
                let n = elements.len();
                let bytes = self.i64t().const_int((n as u64) * 8, false);
                let data = self
                    .builder
                    .build_call(self.malloc, &[bytes.into()], "buf")
                    .unwrap()
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
                    .into_pointer_value();
                for (i, element) in elements.iter().enumerate() {
                    let v = self.lower_int(element)?;
                    let ptr = unsafe {
                        self.builder
                            .build_in_bounds_gep(
                                self.i64t(),
                                data,
                                &[self.i64t().const_int(i as u64, false)],
                                "ep",
                            )
                            .unwrap()
                    };
                    self.builder.build_store(ptr, v).unwrap();
                }
                let arr = self
                    .builder
                    .build_insert_value(
                        array_struct_type(self.context).get_undef(),
                        self.i64t().const_int(n as u64, false),
                        0,
                        "a0",
                    )
                    .unwrap();
                let arr = self
                    .builder
                    .build_insert_value(arr, data, 1, "a1")
                    .unwrap()
                    .into_struct_value();
                Ok((arr.into(), ZType::Array(Box::new(ZType::Int))))
            }
            MirExpr::Index { base, index } => {
                let (base_val, base_ty) = self.lower_expr(base)?;
                let ZType::Array(elem) = base_ty else {
                    return Err("index of non-array".into());
                };
                let data = self
                    .builder
                    .build_extract_value(base_val.into_struct_value(), 1, "data")
                    .unwrap()
                    .into_pointer_value();
                let idx = self.lower_int(index)?;
                let ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(self.i64t(), data, &[idx], "ep")
                        .unwrap()
                };
                let value = self.builder.build_load(self.i64t(), ptr, "elem").unwrap();
                Ok((value, *elem))
            }
            MirExpr::String(text) => {
                // Immutable bytes → a private global constant; the value is
                // `{ byte_len, ptr-to-global }`. `build_global_string_ptr` appends a
                // NUL, but `len` excludes it (matches the interpreter's byte count).
                let global = self.builder.build_global_string_ptr(text, "str").unwrap();
                let data = global.as_pointer_value();
                let len = self.i64t().const_int(text.len() as u64, false);
                Ok((self.make_str(len, data).into(), ZType::Str))
            }
            MirExpr::EnumVariant {
                enum_name,
                variant,
                payload,
            } => {
                // {tag, payload}: tag = variant index; payload = the Int (or 0 for
                // payload-less variants). Non-Int payloads aren't in the E1 subset
                // (lower_int rejects them).
                let (tag, _) = self.types.variant_tag(enum_name, variant)?;
                let payload_val = match payload {
                    Some(expr) => self.lower_int(expr)?,
                    None => self.i64t().const_zero(),
                };
                let v = self
                    .builder
                    .build_insert_value(
                        enum_struct_type(self.context).get_undef(),
                        self.i64t().const_int(tag, false),
                        0,
                        "e0",
                    )
                    .unwrap();
                let v = self
                    .builder
                    .build_insert_value(v, payload_val, 1, "e1")
                    .unwrap()
                    .into_struct_value();
                Ok((v.into(), ZType::Enum(enum_name.clone())))
            }
        }
    }

    /// Lower a std builtin call, or `Ok(None)` if `callee` is not a builtin (then
    /// it is a user function). Strings are `{len, ptr<i8>}`; see `runtime.rs` for
    /// the differential-oracle semantics each builtin must match.
    fn lower_builtin(
        &mut self,
        callee: &str,
        args: &[MirExpr],
    ) -> Result<Option<(BasicValueEnum<'ctx>, ZType)>, String> {
        let b = self.builder;
        match callee {
            "string_len" => {
                let (s, _) = self.lower_expr(&args[0])?;
                let len = b
                    .build_extract_value(s.into_struct_value(), 0, "slen")
                    .unwrap();
                Ok(Some((len, ZType::Int)))
            }
            "string_byte_at" => {
                // `data[index]` as an unsigned byte zero-extended to i64 (the
                // interpreter does `i64::from(u8)`). No bounds check, matching the
                // array path; tests only index in range.
                let (s, _) = self.lower_expr(&args[0])?;
                let data = b
                    .build_extract_value(s.into_struct_value(), 1, "sdata")
                    .unwrap()
                    .into_pointer_value();
                let idx = self.lower_int(&args[1])?;
                let i8t = self.context.i8_type();
                let ptr = unsafe { b.build_in_bounds_gep(i8t, data, &[idx], "bp").unwrap() };
                let byte = b.build_load(i8t, ptr, "byte").unwrap().into_int_value();
                let widened = b.build_int_z_extend(byte, self.i64t(), "byte64").unwrap();
                Ok(Some((widened.into(), ZType::Int)))
            }
            "string_concat" => {
                // malloc(la+lb), memcpy a then b, return {la+lb, buf}.
                let (a, _) = self.lower_expr(&args[0])?;
                let (bv, _) = self.lower_expr(&args[1])?;
                let (la, pa) = self.str_parts(a.into_struct_value());
                let (lb, pb) = self.str_parts(bv.into_struct_value());
                let total = b.build_int_add(la, lb, "clen").unwrap();
                let buf = self.malloc_bytes(total);
                self.memcpy_bytes(buf, pa, la);
                let i8t = self.context.i8_type();
                let tail = unsafe { b.build_in_bounds_gep(i8t, buf, &[la], "tail").unwrap() };
                self.memcpy_bytes(tail, pb, lb);
                Ok(Some((self.make_str(total, buf).into(), ZType::Str)))
            }
            "string_byte_slice" => {
                // s[start .. start+len] → malloc(len) + memcpy from data+start. No
                // bounds/utf-8 check (the array path is likewise unchecked); tests
                // stay in range.
                let (s, _) = self.lower_expr(&args[0])?;
                let (_, data) = self.str_parts(s.into_struct_value());
                let start = self.lower_int(&args[1])?;
                let len = self.lower_int(&args[2])?;
                let i8t = self.context.i8_type();
                let src = unsafe { b.build_in_bounds_gep(i8t, data, &[start], "slcsrc").unwrap() };
                let buf = self.malloc_bytes(len);
                self.memcpy_bytes(buf, src, len);
                Ok(Some((self.make_str(len, buf).into(), ZType::Str)))
            }
            "int_to_string" => {
                // snprintf(buf, 24, "%lld", n): 24 ≥ 20 digits + sign + NUL for any
                // i64. The i32 return is the byte length (excl. NUL), our `len`.
                let n = self.lower_int(&args[0])?;
                let fmt = b.build_global_string_ptr("%lld", "fmt").unwrap().as_pointer_value();
                let cap = self.i64t().const_int(24, false);
                let buf = self.malloc_bytes(cap);
                let written = b
                    .build_call(self.snprintf, &[buf.into(), cap.into(), fmt.into(), n.into()], "snp")
                    .unwrap()
                    .try_as_basic_value()
                    .basic()
                    .unwrap()
                    .into_int_value();
                let len = b.build_int_s_extend(written, self.i64t(), "len64").unwrap();
                Ok(Some((self.make_str(len, buf).into(), ZType::Str)))
            }
            // ascii predicates: Int byte → Bool (i64 0/1). Out-of-[0,255] inputs
            // fall outside every range/equality, yielding 0 — matching the
            // interpreter's explicit `(0..=255)` guard.
            "ascii_is_digit" => {
                let v = self.lower_int(&args[0])?;
                let r = self.in_range(v, 48, 57);
                Ok(Some((self.bool_to_i64(r).into(), ZType::Int)))
            }
            "ascii_is_alpha" => {
                let v = self.lower_int(&args[0])?;
                let r = self.is_alpha(v);
                Ok(Some((self.bool_to_i64(r).into(), ZType::Int)))
            }
            "ascii_is_alnum" => {
                let v = self.lower_int(&args[0])?;
                let digit = self.in_range(v, 48, 57);
                let alpha = self.is_alpha(v);
                let r = b.build_or(digit, alpha, "alnum").unwrap();
                Ok(Some((self.bool_to_i64(r).into(), ZType::Int)))
            }
            "ascii_is_whitespace" => {
                // Rust is_ascii_whitespace: ' '(32) \t(9) \n(10) FF(12) \r(13);
                // note 0x0B (vertical tab) is excluded.
                let v = self.lower_int(&args[0])?;
                let mut acc = self.eq_const(v, 32);
                for k in [9, 10, 12, 13] {
                    let e = self.eq_const(v, k);
                    acc = b.build_or(acc, e, "ws").unwrap();
                }
                Ok(Some((self.bool_to_i64(acc).into(), ZType::Int)))
            }
            _ => Ok(None),
        }
    }

    /// `lo <= v <= hi` (signed) as an i1.
    fn in_range(&self, v: IntValue<'ctx>, lo: i64, hi: i64) -> IntValue<'ctx> {
        let b = self.builder;
        let ge = b
            .build_int_compare(IntPredicate::SGE, v, self.i64t().const_int(lo as u64, true), "ge")
            .unwrap();
        let le = b
            .build_int_compare(IntPredicate::SLE, v, self.i64t().const_int(hi as u64, true), "le")
            .unwrap();
        b.build_and(ge, le, "rng").unwrap()
    }

    /// `v == k` as an i1.
    fn eq_const(&self, v: IntValue<'ctx>, k: i64) -> IntValue<'ctx> {
        self.builder
            .build_int_compare(IntPredicate::EQ, v, self.i64t().const_int(k as u64, true), "eqk")
            .unwrap()
    }

    /// ASCII alphabetic: `A-Z` or `a-z`.
    fn is_alpha(&self, v: IntValue<'ctx>) -> IntValue<'ctx> {
        let upper = self.in_range(v, 65, 90);
        let lower = self.in_range(v, 97, 122);
        self.builder.build_or(upper, lower, "alpha").unwrap()
    }

    /// Zero-extend an i1 to the i64 Bool representation (0/1).
    fn bool_to_i64(&self, bit: IntValue<'ctx>) -> IntValue<'ctx> {
        self.builder.build_int_z_extend(bit, self.i64t(), "b64").unwrap()
    }

    /// Extract `(len, data)` from a string `{i64, ptr}` value.
    fn str_parts(
        &self,
        s: inkwell::values::StructValue<'ctx>,
    ) -> (IntValue<'ctx>, PointerValue<'ctx>) {
        let b = self.builder;
        let len = b.build_extract_value(s, 0, "slen").unwrap().into_int_value();
        let data = b.build_extract_value(s, 1, "sdata").unwrap().into_pointer_value();
        (len, data)
    }

    /// `malloc(n)` returning the i8 buffer pointer.
    fn malloc_bytes(&self, n: IntValue<'ctx>) -> PointerValue<'ctx> {
        self.builder
            .build_call(self.malloc, &[n.into()], "buf")
            .unwrap()
            .try_as_basic_value()
            .basic()
            .unwrap()
            .into_pointer_value()
    }

    /// `memcpy(dst, src, n)`.
    fn memcpy_bytes(&self, dst: PointerValue<'ctx>, src: PointerValue<'ctx>, n: IntValue<'ctx>) {
        self.builder
            .build_call(self.memcpy, &[dst.into(), src.into(), n.into()], "cp")
            .unwrap();
    }

    /// Build a string `{len, data}` value.
    fn make_str(&self, len: IntValue<'ctx>, data: PointerValue<'ctx>) -> inkwell::values::StructValue<'ctx> {
        let b = self.builder;
        let v = b
            .build_insert_value(array_struct_type(self.context).get_undef(), len, 0, "s0")
            .unwrap();
        b.build_insert_value(v, data, 1, "s1").unwrap().into_struct_value()
    }

    /// Lower an expression that must be an `i64` (Int/Bool).
    fn lower_int(&mut self, expr: &MirExpr) -> Result<IntValue<'ctx>, String> {
        let (v, zt) = self.lower_expr(expr)?;
        if zt != ZType::Int {
            return Err("expected Int/Bool value".into());
        }
        Ok(v.into_int_value())
    }

    fn lower_binary(
        &mut self,
        op: BinaryOp,
        left: &MirExpr,
        right: &MirExpr,
    ) -> Result<IntValue<'ctx>, String> {
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            return self.lower_logical(op, left, right);
        }
        let l = self.lower_int(left)?;
        let r = self.lower_int(right)?;
        let b = self.builder;
        Ok(match op {
            BinaryOp::Add => b.build_int_add(l, r, "add").unwrap(),
            BinaryOp::Sub => b.build_int_sub(l, r, "sub").unwrap(),
            BinaryOp::Mul => b.build_int_mul(l, r, "mul").unwrap(),
            BinaryOp::Div => b.build_int_signed_div(l, r, "div").unwrap(),
            BinaryOp::Mod => b.build_int_signed_rem(l, r, "mod").unwrap(),
            BinaryOp::BitAnd => b.build_and(l, r, "band").unwrap(),
            BinaryOp::BitOr => b.build_or(l, r, "bor").unwrap(),
            BinaryOp::BitXor => b.build_xor(l, r, "bxor").unwrap(),
            BinaryOp::Eq => self.compare(IntPredicate::EQ, l, r),
            BinaryOp::NotEq => self.compare(IntPredicate::NE, l, r),
            BinaryOp::Lt => self.compare(IntPredicate::SLT, l, r),
            BinaryOp::Lte => self.compare(IntPredicate::SLE, l, r),
            BinaryOp::Gt => self.compare(IntPredicate::SGT, l, r),
            BinaryOp::Gte => self.compare(IntPredicate::SGE, l, r),
            BinaryOp::And | BinaryOp::Or => unreachable!(),
        })
    }

    fn compare(&self, pred: IntPredicate, l: IntValue<'ctx>, r: IntValue<'ctx>) -> IntValue<'ctx> {
        let bit = self.builder.build_int_compare(pred, l, r, "cmp").unwrap();
        self.builder.build_int_z_extend(bit, self.i64t(), "cmp64").unwrap()
    }

    fn lower_logical(
        &mut self,
        op: BinaryOp,
        left: &MirExpr,
        right: &MirExpr,
    ) -> Result<IntValue<'ctx>, String> {
        let result = self.entry_alloca("logic", self.i64t().into());
        let l = self.lower_int(left)?;
        let l_bool = self
            .builder
            .build_int_compare(IntPredicate::NE, l, self.i64t().const_zero(), "lb")
            .unwrap();
        let rhs_bb = self.context.append_basic_block(self.llvm_fn, "logic.rhs");
        let short_bb = self.context.append_basic_block(self.llvm_fn, "logic.short");
        let cont_bb = self.context.append_basic_block(self.llvm_fn, "logic.cont");
        match op {
            BinaryOp::And => self
                .builder
                .build_conditional_branch(l_bool, rhs_bb, short_bb)
                .unwrap(),
            BinaryOp::Or => self
                .builder
                .build_conditional_branch(l_bool, short_bb, rhs_bb)
                .unwrap(),
            _ => unreachable!(),
        };
        self.builder.position_at_end(short_bb);
        let short_value = if matches!(op, BinaryOp::And) {
            self.i64t().const_zero()
        } else {
            self.i64t().const_int(1, false)
        };
        self.builder.build_store(result, short_value).unwrap();
        self.builder.build_unconditional_branch(cont_bb).unwrap();
        self.builder.position_at_end(rhs_bb);
        let r = self.lower_int(right)?;
        let r_bool = self
            .builder
            .build_int_compare(IntPredicate::NE, r, self.i64t().const_zero(), "rb")
            .unwrap();
        let r_i64 = self.builder.build_int_z_extend(r_bool, self.i64t(), "rb64").unwrap();
        self.builder.build_store(result, r_i64).unwrap();
        self.builder.build_unconditional_branch(cont_bb).unwrap();
        self.builder.position_at_end(cont_bb);
        Ok(self
            .builder
            .build_load(self.i64t(), result, "logic.val")
            .unwrap()
            .into_int_value())
    }

    fn lower_cond(&mut self, expr: &MirExpr) -> Result<IntValue<'ctx>, String> {
        let v = self.lower_int(expr)?;
        Ok(self
            .builder
            .build_int_compare(IntPredicate::NE, v, self.i64t().const_zero(), "tobool")
            .unwrap())
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
