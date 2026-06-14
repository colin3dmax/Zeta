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
//! locals/params/returns, nesting. Arrays/strings/enums/match/for still `Err`.

use crate::ast::{BinaryOp, StructDecl, UnaryOp};
use crate::mir::{MirExpr, MirPlace, MirStmt, Program};
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
                let zt = parse_ztype(&field.ty, &names)?;
                field_llvm.push(llvm_type_of(context, &zt, &opaque));
                fields.push((field.name.clone(), zt));
            }
            let ty = opaque[&decl.name];
            ty.set_body(&field_llvm, false);
            structs.insert(decl.name.clone(), StructInfo { fields, ty });
        }

        let mut returns = HashMap::new();
        for function in &program.functions {
            let zt = match &function.return_type {
                Some(t) => parse_ztype(t, &names).unwrap_or(ZType::Int),
                None => ZType::Int, // Unit-returning → i64 0
            };
            returns.insert(function.name.clone(), zt);
        }

        Ok(Types {
            context,
            structs,
            returns,
        })
    }

    fn llvm(&self, zt: &ZType) -> BasicTypeEnum<'ctx> {
        match zt {
            ZType::Int => self.context.i64_type().into(),
            ZType::Struct(name) => self.structs[name].ty.into(),
            ZType::Array(_) => array_struct_type(self.context).into(),
        }
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

fn parse_ztype(text: &str, struct_names: &[&str]) -> Result<ZType, String> {
    match text {
        "Int" | "Bool" => Ok(ZType::Int),
        "Unit" => Ok(ZType::Int),
        "IntArray" => Ok(ZType::Array(Box::new(ZType::Int))),
        name if struct_names.contains(&name) => Ok(ZType::Struct(name.to_string())),
        other => Err(format!("type `{other}` not in the native subset")),
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

fn llvm_type_of<'ctx>(
    context: &'ctx Context,
    zt: &ZType,
    opaque: &HashMap<String, StructType<'ctx>>,
) -> BasicTypeEnum<'ctx> {
    match zt {
        ZType::Int => context.i64_type().into(),
        ZType::Struct(name) => opaque[name].into(),
        ZType::Array(_) => array_struct_type(context).into(),
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

fn build_module<'ctx>(
    context: &'ctx Context,
    types: &Types<'ctx>,
    program: &Program,
) -> Result<inkwell::module::Module<'ctx>, String> {
    let module = context.create_module("zeta_native");
    let builder = context.create_builder();
    let struct_names: Vec<&str> = types.structs.keys().map(|s| s.as_str()).collect();

    // libc malloc/memcpy for array buffers + deep copies (link via libc).
    let ptr_ty = context.ptr_type(inkwell::AddressSpace::default());
    let i64_ty = context.i64_type();
    let malloc = module.add_function("malloc", ptr_ty.fn_type(&[i64_ty.into()], false), None);
    let memcpy = module.add_function(
        "memcpy",
        ptr_ty.fn_type(&[ptr_ty.into(), ptr_ty.into(), i64_ty.into()], false),
        None,
    );

    // Pass 1: declare every function with its typed signature.
    let mut functions: HashMap<String, FunctionValue> = HashMap::new();
    for function in &program.functions {
        let mut param_types = Vec::with_capacity(function.params.len());
        for param in &function.params {
            let zt = parse_ztype(&param.ty, &struct_names)?;
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
            env.insert(param.name.clone(), parse_ztype(&param.ty, &struct_names)?);
        }
        infer_locals(&function.body, types, &struct_names, &mut env)?;

        let mut lower = FnLower {
            context,
            builder: &builder,
            types,
            functions: &functions,
            malloc,
            memcpy,
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
    env: &mut HashMap<String, ZType>,
) -> Result<(), String> {
    for stmt in stmts {
        match stmt {
            MirStmt::Local {
                name, ty, value, ..
            } => {
                let zt = match ty {
                    Some(t) => parse_ztype(t, struct_names)?,
                    None => infer_expr_type(value, types, env)?,
                };
                env.insert(name.clone(), zt);
            }
            MirStmt::If {
                then_body,
                else_body,
                ..
            } => {
                infer_locals(then_body, types, struct_names, env)?;
                infer_locals(else_body, types, struct_names, env)?;
            }
            MirStmt::While { body, .. } => infer_locals(body, types, struct_names, env)?,
            _ => {}
        }
    }
    Ok(())
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
        MirExpr::Call { callee, .. } => types
            .returns
            .get(callee)
            .cloned()
            .ok_or_else(|| format!("unknown function `{callee}`"))?,
        _ => return Err("expression not in the native subset".into()),
    })
}

fn optimize_module(module: &inkwell::module::Module) -> Result<(), String> {
    use inkwell::targets::{CodeModel, InitializationConfig, RelocMode, Target, TargetMachine};

    Target::initialize_native(&InitializationConfig::default())
        .map_err(|e| format!("native target init failed: {e}"))?;
    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple).map_err(|e| format!("target lookup failed: {e}"))?;
    let machine = target
        .create_target_machine(
            &triple,
            TargetMachine::get_host_cpu_name().to_str().unwrap_or(""),
            TargetMachine::get_host_cpu_features().to_str().unwrap_or(""),
            OptimizationLevel::Aggressive,
            RelocMode::Default,
            CodeModel::Default,
        )
        .ok_or("could not create host target machine")?;
    module
        .run_passes(
            "default<O3>",
            &machine,
            inkwell::passes::PassBuilderOptions::create(),
        )
        .map_err(|e| format!("optimization passes failed: {e}"))
}

struct FnLower<'a, 'ctx> {
    context: &'ctx Context,
    builder: &'a Builder<'ctx>,
    types: &'a Types<'ctx>,
    functions: &'a HashMap<String, FunctionValue<'ctx>>,
    malloc: FunctionValue<'ctx>,
    memcpy: FunctionValue<'ctx>,
    llvm_fn: FunctionValue<'ctx>,
    entry_bb: BasicBlock<'ctx>,
    /// local name → (alloca slot, type)
    locals: HashMap<String, (PointerValue<'ctx>, ZType)>,
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
            ZType::Array(_) => array_struct_type(self.context).const_zero().into(),
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
            MirStmt::ForIn { .. }
            | MirStmt::ForRange { .. }
            | MirStmt::ForC { .. }
            | MirStmt::Match { .. } => Err("for/match not in the native subset".into()),
        }
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
            MirExpr::String(_) | MirExpr::EnumVariant { .. } => {
                Err("string/enum expression not in the native subset".into())
            }
        }
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
