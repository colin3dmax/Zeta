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

use crate::ast::{BinaryOp, UnaryOp};
use crate::mir::{MirExpr, MirPlace, MirStmt, Program};
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::values::{FunctionValue, IntValue, PointerValue};
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

/// JIT-compile `program` and run its `entry` function (which must take no
/// parameters and return Int) to a single `i64`. The scalar subset: Int/Bool
/// (both modelled as `i64`), arithmetic/bitwise/comparison/logical operators,
/// unary ops, `let`/assignment of locals, `if`/`while`/`break`/`continue`,
/// user function calls, and `return`. Aggregates (struct/array/enum/match/for/
/// string) return an `Err` for now — they land in the next slice.
///
/// The Stage0 interpreter (`run_mir`) is the differential oracle: for any
/// program in this subset, `jit_run_i64(p, "main")` must equal the `i64` the
/// interpreter produces.
pub fn jit_run_i64(program: &Program, entry: &str) -> Result<i64, String> {
    let context = Context::create();
    let module = context.create_module("zeta_native");
    let builder = context.create_builder();
    let i64_type = context.i64_type();

    // Pass 1: declare every function (so calls resolve regardless of order).
    let mut functions: HashMap<String, FunctionValue> = HashMap::new();
    for function in &program.functions {
        let param_types = vec![i64_type.into(); function.params.len()];
        let fn_type = i64_type.fn_type(&param_types, false);
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

        let mut lower = FnLower {
            context: &context,
            builder: &builder,
            i64_type,
            functions: &functions,
            llvm_fn,
            entry_bb,
            locals: HashMap::new(),
            loops: Vec::new(),
        };
        // Allocate slots for params + all `let`-bound locals, then seed params.
        let mut names: Vec<String> = function.params.iter().map(|p| p.name.clone()).collect();
        collect_local_names(&function.body, &mut names);
        for name in &names {
            let slot = lower.entry_alloca(name);
            lower.locals.insert(name.clone(), slot);
        }
        for (index, param) in function.params.iter().enumerate() {
            let value = llvm_fn
                .get_nth_param(index as u32)
                .expect("param exists")
                .into_int_value();
            builder.build_store(lower.locals[&param.name], value).unwrap();
        }

        let terminated = lower.lower_stmts(&function.body)?;
        if !terminated {
            // Fall-through (e.g. a Unit-returning function): default to 0.
            builder
                .build_return(Some(&i64_type.const_zero()))
                .unwrap();
        }
    }

    module
        .verify()
        .map_err(|e| format!("LLVM module verification failed: {e}"))?;

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

/// Recursively collect the names bound by `let` (MirStmt::Local) anywhere in the
/// scalar control-flow subset, so they can be pre-allocated in the entry block.
fn collect_local_names(stmts: &[MirStmt], out: &mut Vec<String>) {
    for stmt in stmts {
        match stmt {
            MirStmt::Local { name, .. } => {
                if !out.contains(name) {
                    out.push(name.clone());
                }
            }
            MirStmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_local_names(then_body, out);
                collect_local_names(else_body, out);
            }
            MirStmt::While { body, .. } => collect_local_names(body, out),
            _ => {}
        }
    }
}

struct FnLower<'a, 'ctx> {
    context: &'ctx Context,
    builder: &'a Builder<'ctx>,
    i64_type: inkwell::types::IntType<'ctx>,
    functions: &'a HashMap<String, FunctionValue<'ctx>>,
    llvm_fn: FunctionValue<'ctx>,
    entry_bb: BasicBlock<'ctx>,
    locals: HashMap<String, PointerValue<'ctx>>,
    /// Stack of (continue-target, break-target) for the enclosing loops.
    loops: Vec<(BasicBlock<'ctx>, BasicBlock<'ctx>)>,
}

impl<'a, 'ctx> FnLower<'a, 'ctx> {
    /// Allocate an `i64` slot at the TOP of the entry block (so LLVM's mem2reg
    /// can promote it to an SSA register — the key to native-quality code).
    fn entry_alloca(&self, name: &str) -> PointerValue<'ctx> {
        let saved = self.builder.get_insert_block();
        match self.entry_bb.get_first_instruction() {
            Some(first) => self.builder.position_before(&first),
            None => self.builder.position_at_end(self.entry_bb),
        }
        let slot = self.builder.build_alloca(self.i64_type, name).unwrap();
        if let Some(block) = saved {
            self.builder.position_at_end(block);
        }
        slot
    }

    /// Lower a statement list. Returns true if it ended with a terminator
    /// (return/break/continue) so the caller stops emitting into this block.
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
                let value = self.lower_expr(value)?;
                self.builder.build_store(self.locals[name], value).unwrap();
                Ok(false)
            }
            MirStmt::Store {
                place: MirPlace::Local(name),
                value,
            } => {
                let value = self.lower_expr(value)?;
                let slot = *self
                    .locals
                    .get(name)
                    .ok_or_else(|| format!("store to unknown local `{name}`"))?;
                self.builder.build_store(slot, value).unwrap();
                Ok(false)
            }
            MirStmt::Store { .. } => Err("field/index store not in the scalar subset".into()),
            MirStmt::Return(value) => {
                match value {
                    Some(expr) => {
                        let v = self.lower_expr(expr)?;
                        self.builder.build_return(Some(&v)).unwrap();
                    }
                    None => {
                        self.builder
                            .build_return(Some(&self.i64_type.const_zero()))
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
                let header_bb = self.context.append_basic_block(self.llvm_fn, "while.head");
                let body_bb = self.context.append_basic_block(self.llvm_fn, "while.body");
                let exit_bb = self.context.append_basic_block(self.llvm_fn, "while.exit");
                self.builder.build_unconditional_branch(header_bb).unwrap();

                self.builder.position_at_end(header_bb);
                let cond = self.lower_cond(condition)?;
                self.builder
                    .build_conditional_branch(cond, body_bb, exit_bb)
                    .unwrap();

                self.loops.push((header_bb, exit_bb));
                self.builder.position_at_end(body_bb);
                if !self.lower_stmts(body)? {
                    self.builder.build_unconditional_branch(header_bb).unwrap();
                }
                self.loops.pop();

                self.builder.position_at_end(exit_bb);
                Ok(false)
            }
            MirStmt::Break => {
                let (_, exit) = *self
                    .loops
                    .last()
                    .ok_or("`break` outside loop in codegen")?;
                self.builder.build_unconditional_branch(exit).unwrap();
                Ok(true)
            }
            MirStmt::Continue => {
                let (header, _) = *self
                    .loops
                    .last()
                    .ok_or("`continue` outside loop in codegen")?;
                self.builder.build_unconditional_branch(header).unwrap();
                Ok(true)
            }
            MirStmt::Drop(expr) => {
                self.lower_expr(expr)?;
                Ok(false)
            }
            MirStmt::ForIn { .. }
            | MirStmt::ForRange { .. }
            | MirStmt::ForC { .. }
            | MirStmt::Match { .. } => {
                Err("for/match not in the scalar subset".into())
            }
        }
    }

    /// Lower an expression to an `i64` value (Bool is 0/1).
    fn lower_expr(&mut self, expr: &MirExpr) -> Result<IntValue<'ctx>, String> {
        match expr {
            MirExpr::Int(text) => {
                let n: i64 = text
                    .parse()
                    .map_err(|_| format!("invalid Int literal `{text}`"))?;
                Ok(self.i64_type.const_int(n as u64, true))
            }
            MirExpr::Bool(b) => Ok(self.i64_type.const_int(*b as u64, false)),
            MirExpr::Load(name) => {
                let slot = *self
                    .locals
                    .get(name)
                    .ok_or_else(|| format!("load of unknown local `{name}`"))?;
                Ok(self
                    .builder
                    .build_load(self.i64_type, slot, name)
                    .unwrap()
                    .into_int_value())
            }
            MirExpr::Unary { op, expr } => {
                let v = self.lower_expr(expr)?;
                Ok(match op {
                    UnaryOp::Neg => self.builder.build_int_neg(v, "neg").unwrap(),
                    UnaryOp::BitNot => self.builder.build_not(v, "bitnot").unwrap(),
                    UnaryOp::Not => {
                        // logical not: (v == 0) ? 1 : 0
                        let is_zero = self
                            .builder
                            .build_int_compare(IntPredicate::EQ, v, self.i64_type.const_zero(), "isz")
                            .unwrap();
                        self.builder
                            .build_int_z_extend(is_zero, self.i64_type, "not")
                            .unwrap()
                    }
                })
            }
            MirExpr::Binary { op, left, right } => self.lower_binary(*op, left, right),
            MirExpr::Call { callee, args } => {
                let function = *self
                    .functions
                    .get(callee)
                    .ok_or_else(|| format!("call to unsupported/unknown `{callee}`"))?;
                let mut argv = Vec::with_capacity(args.len());
                for arg in args {
                    argv.push(self.lower_expr(arg)?.into());
                }
                let call = self.builder.build_call(function, &argv, "call").unwrap();
                Ok(call
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| format!("call to `{callee}` did not return a value"))?
                    .into_int_value())
            }
            MirExpr::String(_)
            | MirExpr::EnumVariant { .. }
            | MirExpr::StructLiteral { .. }
            | MirExpr::FieldAccess { .. }
            | MirExpr::ArrayLiteral { .. }
            | MirExpr::Index { .. } => {
                Err("aggregate/string expression not in the scalar subset".into())
            }
        }
    }

    fn lower_binary(
        &mut self,
        op: BinaryOp,
        left: &MirExpr,
        right: &MirExpr,
    ) -> Result<IntValue<'ctx>, String> {
        // Short-circuiting logical operators need control flow.
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            return self.lower_logical(op, left, right);
        }
        let l = self.lower_expr(left)?;
        let r = self.lower_expr(right)?;
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
            BinaryOp::And | BinaryOp::Or => unreachable!("handled above"),
        })
    }

    fn compare(&self, pred: IntPredicate, l: IntValue<'ctx>, r: IntValue<'ctx>) -> IntValue<'ctx> {
        let bit = self.builder.build_int_compare(pred, l, r, "cmp").unwrap();
        self.builder
            .build_int_z_extend(bit, self.i64_type, "cmp64")
            .unwrap()
    }

    /// Short-circuiting `&&` / `||`, result as 0/1 `i64`. Uses an entry-block
    /// slot so mem2reg promotes it.
    fn lower_logical(
        &mut self,
        op: BinaryOp,
        left: &MirExpr,
        right: &MirExpr,
    ) -> Result<IntValue<'ctx>, String> {
        let result = self.entry_alloca("logic");
        let l = self.lower_expr(left)?;
        let l_bool = self
            .builder
            .build_int_compare(IntPredicate::NE, l, self.i64_type.const_zero(), "lb")
            .unwrap();

        let rhs_bb = self.context.append_basic_block(self.llvm_fn, "logic.rhs");
        let short_bb = self.context.append_basic_block(self.llvm_fn, "logic.short");
        let cont_bb = self.context.append_basic_block(self.llvm_fn, "logic.cont");

        match op {
            // a && b: if a -> eval b, else short-circuit to 0.
            BinaryOp::And => self
                .builder
                .build_conditional_branch(l_bool, rhs_bb, short_bb)
                .unwrap(),
            // a || b: if a -> short-circuit to 1, else eval b.
            BinaryOp::Or => self
                .builder
                .build_conditional_branch(l_bool, short_bb, rhs_bb)
                .unwrap(),
            _ => unreachable!(),
        };

        self.builder.position_at_end(short_bb);
        let short_value = if matches!(op, BinaryOp::And) {
            self.i64_type.const_zero()
        } else {
            self.i64_type.const_int(1, false)
        };
        self.builder.build_store(result, short_value).unwrap();
        self.builder.build_unconditional_branch(cont_bb).unwrap();

        self.builder.position_at_end(rhs_bb);
        let r = self.lower_expr(right)?;
        let r_bool = self
            .builder
            .build_int_compare(IntPredicate::NE, r, self.i64_type.const_zero(), "rb")
            .unwrap();
        let r_i64 = self
            .builder
            .build_int_z_extend(r_bool, self.i64_type, "rb64")
            .unwrap();
        self.builder.build_store(result, r_i64).unwrap();
        self.builder.build_unconditional_branch(cont_bb).unwrap();

        self.builder.position_at_end(cont_bb);
        Ok(self
            .builder
            .build_load(self.i64_type, result, "logic.val")
            .unwrap()
            .into_int_value())
    }

    /// Lower a condition expression to an `i1` (compares the i64 value to 0).
    fn lower_cond(&mut self, expr: &MirExpr) -> Result<IntValue<'ctx>, String> {
        let v = self.lower_expr(expr)?;
        Ok(self
            .builder
            .build_int_compare(IntPredicate::NE, v, self.i64_type.const_zero(), "tobool")
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
