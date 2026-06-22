//! Move-on-last-use analysis: backward liveness over the structured MIR to find
//! `Load(x)` reads that are the LAST use of a managed local on every path. At
//! such a read the codegen may MOVE ownership (skip the value-semantics clone)
//! instead of copying, provided the local's scope-exit drop is suppressed when —
//! and only when — the move actually executed (a runtime "moved" flag handles
//! the path-sensitive case; see `codegen`).
//!
//! Soundness is the whole game here, so the analysis is deliberately
//! conservative — it only ever marks a read when it can prove no later read of
//! the same local is reachable (including across loop back-edges):
//!
//!   * Eligible reads are restricted to ownership-TRANSFERRING positions (call
//!     argument, aggregate member, `let`/assignment RHS, enum payload, return
//!     operand). A `Load` that is the base of an index/field read merely borrows
//!     the container and is never moved.
//!   * A name is blacklisted entirely if name-based liveness cannot model it:
//!     shadowed (defined more than once), introduced by a `for`/`match` binding,
//!     or captured by a lambda.
//!   * A function containing `break`/`continue` is opted out wholesale — those
//!     transfer control in ways this simple per-block backward walk does not
//!     model, and a wrong liveness fact there would be a use-after-free.
//!
//! The result identifies eligible reads by the ADDRESS of their `MirExpr` node,
//! which is stable because codegen borrows the `Program` immutably throughout.
//! A generic body lowered at several types reuses the same nodes, and since the
//! analysis is purely name-based (type-independent) the same decisions correctly
//! apply to every monomorphization; codegen additionally gates each move on the
//! value actually being a managed (droppable) type.

use std::collections::HashSet;

use crate::mir::{MirExpr, MirFunction, MirStmt, MirPlace};

/// The set of `Load` nodes a function may move from, plus the names that carry a
/// runtime moved-flag (every local with at least one eligible read).
pub struct MovePlan {
    eligible: HashSet<usize>,
    flagged: HashSet<String>,
}

impl MovePlan {
    /// A plan that permits no moves — used where analysis is skipped (e.g. the
    /// body of a lifted lambda).
    pub fn empty() -> Self {
        MovePlan { eligible: HashSet::new(), flagged: HashSet::new() }
    }

    pub fn is_move(&self, expr: &MirExpr) -> bool {
        self.eligible.contains(&(expr as *const MirExpr as usize))
    }

    /// Whether `name` needs a runtime moved-flag (it is moved somewhere).
    pub fn is_flagged(&self, name: &str) -> bool {
        self.flagged.contains(name)
    }
}

/// Build the move plan for one function (a specialization template is analyzed
/// once; its decisions hold for every instantiation).
pub fn analyze(func: &MirFunction) -> MovePlan {
    // Opt the whole function out if it uses non-local control flow we don't model.
    if contains_loop_jump(&func.body) {
        return MovePlan { eligible: HashSet::new(), flagged: HashSet::new() };
    }
    let blacklist = blacklist(func);
    let mut moves = HashSet::new();
    let mut names = HashSet::new();
    let mut a = Analyzer { blacklist: &blacklist, moves: &mut moves, names: &mut names };
    a.walk_stmts(&func.body, HashSet::new(), true);
    MovePlan { eligible: moves, flagged: names }
}

struct Analyzer<'a> {
    blacklist: &'a HashSet<String>,
    moves: &'a mut HashSet<usize>,
    names: &'a mut HashSet<String>,
}

/// Backward-liveness traversal that records both the eligible `Load` node
/// addresses and the names that carry a moved-flag.
macro_rules! impl_walk {
    ($t:ident, $record_move:expr) => {
        impl<'a> $t<'a> {
            /// Backward liveness over a statement sequence. `record` is false
            /// during loop fixpoint iterations (so a read is only committed once
            /// the loop's live-out has stabilized).
            fn walk_stmts(
                &mut self,
                stmts: &[MirStmt],
                live_out: HashSet<String>,
                record: bool,
            ) -> HashSet<String> {
                let mut live = live_out;
                for stmt in stmts.iter().rev() {
                    live = self.walk_stmt(stmt, live, record);
                }
                live
            }

            fn walk_stmt(
                &mut self,
                stmt: &MirStmt,
                live_out: HashSet<String>,
                record: bool,
            ) -> HashSet<String> {
                match stmt {
                    MirStmt::Local { name, value, .. } => {
                        let mut after = live_out;
                        after.remove(name); // the def kills any downstream liveness
                        self.walk_expr(value, after, true, record)
                    }
                    MirStmt::Store { place, value } => {
                        // RHS is consumed into the place; the place's own index
                        // expressions and base are reads that keep their vars live.
                        let live = self.walk_place(place, live_out, record);
                        self.walk_expr(value, live, true, record)
                    }
                    MirStmt::If { condition, then_body, else_body } => {
                        let lt = self.walk_stmts(then_body, live_out.clone(), record);
                        let le = self.walk_stmts(else_body, live_out, record);
                        let merged = union(lt, le);
                        self.walk_expr(condition, merged, false, record)
                    }
                    MirStmt::While { condition, body } => {
                        // Fixpoint without recording, then a single recording pass
                        // over the body using the stabilized live-out.
                        let mut lbo = live_out.clone();
                        loop {
                            let li_body = self.walk_stmts(body, lbo.clone(), false);
                            let li_top = self.walk_expr(
                                condition,
                                union(li_body, live_out.clone()),
                                false,
                                false,
                            );
                            if li_top.is_subset(&lbo) {
                                break;
                            }
                            lbo = union(lbo, li_top);
                        }
                        if record {
                            self.walk_stmts(body, lbo.clone(), true);
                        }
                        self.walk_expr(condition, lbo, false, false)
                    }
                    MirStmt::Return(value) => match value {
                        // After a return nothing is live; statements lexically
                        // after it are dead, so we discard `live_out`.
                        Some(expr) => self.walk_expr(expr, HashSet::new(), true, record),
                        None => HashSet::new(),
                    },
                    // `for`/`match` introduce bindings handled via the blacklist;
                    // their sub-expressions are still scanned for liveness, with no
                    // moves recorded inside (conservative).
                    MirStmt::ForIn { iterable, body, .. } => {
                        let _ = self.walk_stmts(body, live_out.clone(), false);
                        self.walk_expr(iterable, live_out, false, false)
                    }
                    MirStmt::ForRange { start, end, body, .. } => {
                        let _ = self.walk_stmts(body, live_out.clone(), false);
                        let live = self.walk_expr(end, live_out, false, false);
                        self.walk_expr(start, live, false, false)
                    }
                    MirStmt::ForC { init, condition, step, body } => {
                        let _ = self.walk_stmts(body, live_out.clone(), false);
                        let _ = self.walk_stmt(step, live_out.clone(), false);
                        let live = self.walk_expr(condition, live_out, false, false);
                        self.walk_stmt(init, live, false)
                    }
                    MirStmt::Match { value, arms } => {
                        let mut merged = HashSet::new();
                        for arm in arms {
                            merged = union(merged, self.walk_stmts(&arm.body, live_out.clone(), false));
                        }
                        self.walk_expr(value, merged, false, false)
                    }
                    MirStmt::Drop(expr) => self.walk_expr(expr, live_out, false, record),
                    // Unreachable here: `break`/`continue` opt the function out.
                    MirStmt::Break | MirStmt::Continue => live_out,
                }
            }

            /// Reads performed by an assignment place (index expressions plus the
            /// base local) — all keep their vars live; none are moves.
            fn walk_place(
                &mut self,
                place: &MirPlace,
                live_out: HashSet<String>,
                record: bool,
            ) -> HashSet<String> {
                match place {
                    MirPlace::Local(name) => {
                        // A simple reassignment target is conservatively treated as
                        // still-live (not killed), which only ever forbids moves.
                        let mut live = live_out;
                        live.insert(name.clone());
                        live
                    }
                    MirPlace::Field { base, .. } => self.walk_place(base, live_out, record),
                    MirPlace::Index { base, index } => {
                        let live = self.walk_expr(index, live_out, false, record);
                        self.walk_place(base, live, record)
                    }
                }
            }

            /// Backward liveness over an expression. `consuming` marks ownership-
            /// transferring positions where a last-use `Load` may be moved.
            fn walk_expr(
                &mut self,
                expr: &MirExpr,
                live_out: HashSet<String>,
                consuming: bool,
                record: bool,
            ) -> HashSet<String> {
                match expr {
                    MirExpr::Load(name) => {
                        let dead_after = !live_out.contains(name);
                        if record
                            && consuming
                            && dead_after
                            && !self.blacklist.contains(name)
                        {
                            $record_move(self, expr, name);
                        }
                        let mut live = live_out;
                        live.insert(name.clone());
                        live
                    }
                    MirExpr::Int(_) | MirExpr::Float(_) | MirExpr::String(_) | MirExpr::Bool(_) => {
                        live_out
                    }
                    MirExpr::Binary { left, right, .. } => {
                        let live = self.walk_expr(right, live_out, false, record);
                        self.walk_expr(left, live, false, record)
                    }
                    MirExpr::Unary { expr, .. } => self.walk_expr(expr, live_out, false, record),
                    MirExpr::Call { args, .. } => {
                        let mut live = live_out;
                        for arg in args.iter().rev() {
                            live = self.walk_expr(arg, live, true, record);
                        }
                        live
                    }
                    MirExpr::EnumVariant { payload, .. } => match payload {
                        Some(p) => self.walk_expr(p, live_out, true, record),
                        None => live_out,
                    },
                    MirExpr::StructLiteral { fields, .. } => {
                        let mut live = live_out;
                        for field in fields.iter().rev() {
                            live = self.walk_expr(&field.value, live, true, record);
                        }
                        live
                    }
                    MirExpr::ArrayLiteral { elements } | MirExpr::Tuple { elements } => {
                        let mut live = live_out;
                        for elem in elements.iter().rev() {
                            live = self.walk_expr(elem, live, true, record);
                        }
                        live
                    }
                    MirExpr::FieldAccess { base, .. } => {
                        self.walk_expr(base, live_out, false, record)
                    }
                    MirExpr::Index { base, index } => {
                        let live = self.walk_expr(index, live_out, false, record);
                        self.walk_expr(base, live, false, record)
                    }
                    MirExpr::Lambda { body, .. } => {
                        // Captured free vars stay live; nothing inside is moved.
                        self.walk_expr(body, live_out, false, false)
                    }
                }
            }
        }
    };
}

impl_walk!(Analyzer, |s: &mut Analyzer, expr: &MirExpr, name: &str| {
    s.moves.insert(expr as *const MirExpr as usize);
    s.names.insert(name.to_string());
});

/// Union of two live-sets (consumes both).
fn union(mut a: HashSet<String>, b: HashSet<String>) -> HashSet<String> {
    a.extend(b);
    a
}

/// True if any loop in the body uses `break`/`continue`, which opts the whole
/// function out of move analysis.
fn contains_loop_jump(stmts: &[MirStmt]) -> bool {
    stmts.iter().any(stmt_has_jump)
}

fn stmt_has_jump(stmt: &MirStmt) -> bool {
    match stmt {
        MirStmt::Break | MirStmt::Continue => true,
        MirStmt::If { then_body, else_body, .. } => {
            contains_loop_jump(then_body) || contains_loop_jump(else_body)
        }
        MirStmt::While { body, .. }
        | MirStmt::ForIn { body, .. }
        | MirStmt::ForRange { body, .. } => contains_loop_jump(body),
        MirStmt::ForC { init, step, body, .. } => {
            stmt_has_jump(init) || stmt_has_jump(step) || contains_loop_jump(body)
        }
        MirStmt::Match { arms, .. } => arms.iter().any(|a| contains_loop_jump(&a.body)),
        _ => false,
    }
}

/// Names that name-based liveness cannot soundly track: defined more than once
/// (shadowing), introduced by a `for`/`match` binding, or captured by a lambda.
fn blacklist(func: &MirFunction) -> HashSet<String> {
    let mut counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for param in &func.params {
        *counts.entry(param.name.clone()).or_default() += 1;
    }
    let mut forced = HashSet::new();
    collect_bindings(&func.body, &mut counts, &mut forced);
    collect_captures(&func.body, &mut forced);
    let mut bl = forced;
    for (name, n) in counts {
        if n > 1 {
            bl.insert(name);
        }
    }
    bl
}

fn collect_bindings(
    stmts: &[MirStmt],
    counts: &mut std::collections::HashMap<String, u32>,
    forced: &mut HashSet<String>,
) {
    for stmt in stmts {
        match stmt {
            MirStmt::Local { name, .. } => {
                *counts.entry(name.clone()).or_default() += 1;
            }
            MirStmt::If { then_body, else_body, .. } => {
                collect_bindings(then_body, counts, forced);
                collect_bindings(else_body, counts, forced);
            }
            MirStmt::While { body, .. } => collect_bindings(body, counts, forced),
            MirStmt::ForIn { binding, body, .. } => {
                forced.insert(binding.clone());
                collect_bindings(body, counts, forced);
            }
            MirStmt::ForRange { binding, body, .. } => {
                forced.insert(binding.clone());
                collect_bindings(body, counts, forced);
            }
            MirStmt::ForC { init, step, body, .. } => {
                collect_bindings(std::slice::from_ref(&**init), counts, forced);
                collect_bindings(std::slice::from_ref(&**step), counts, forced);
                collect_bindings(body, counts, forced);
            }
            MirStmt::Match { arms, .. } => {
                for arm in arms {
                    if let crate::mir::MirPattern::Variant { binding: Some(b), .. } = &arm.pattern {
                        forced.insert(b.clone());
                    }
                    if let crate::mir::MirPattern::Name(b) = &arm.pattern {
                        forced.insert(b.clone());
                    }
                    collect_bindings(&arm.body, counts, forced);
                }
            }
            _ => {}
        }
    }
}

fn collect_captures(stmts: &[MirStmt], forced: &mut HashSet<String>) {
    for stmt in stmts {
        walk_stmt_exprs(stmt, &mut |expr| collect_lambda_free_vars(expr, forced));
    }
}

/// If `expr` is a lambda, add the free variables it captures to `forced`. Any
/// `Load` not bound by the lambda's own params is treated as captured (an
/// over-approximation is fine — it only ever adds blacklist entries).
fn collect_lambda_free_vars(expr: &MirExpr, forced: &mut HashSet<String>) {
    if let MirExpr::Lambda { params, body } = expr {
        let bound: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
        walk_expr_tree(body, &mut |e| {
            if let MirExpr::Load(name) = e {
                if !bound.contains(name) {
                    forced.insert(name.clone());
                }
            }
        });
    }
}

/// Visit every expression in a statement (shallowly recursing into nested
/// statement bodies), applying `f`.
fn walk_stmt_exprs(stmt: &MirStmt, f: &mut dyn FnMut(&MirExpr)) {
    match stmt {
        MirStmt::Local { value, .. } => walk_expr_tree(value, f),
        MirStmt::Store { place, value } => {
            walk_place_exprs(place, f);
            walk_expr_tree(value, f);
        }
        MirStmt::If { condition, then_body, else_body } => {
            walk_expr_tree(condition, f);
            for s in then_body {
                walk_stmt_exprs(s, f);
            }
            for s in else_body {
                walk_stmt_exprs(s, f);
            }
        }
        MirStmt::While { condition, body } => {
            walk_expr_tree(condition, f);
            for s in body {
                walk_stmt_exprs(s, f);
            }
        }
        MirStmt::ForIn { iterable, body, .. } => {
            walk_expr_tree(iterable, f);
            for s in body {
                walk_stmt_exprs(s, f);
            }
        }
        MirStmt::ForRange { start, end, body, .. } => {
            walk_expr_tree(start, f);
            walk_expr_tree(end, f);
            for s in body {
                walk_stmt_exprs(s, f);
            }
        }
        MirStmt::ForC { init, condition, step, body } => {
            walk_stmt_exprs(init, f);
            walk_expr_tree(condition, f);
            walk_stmt_exprs(step, f);
            for s in body {
                walk_stmt_exprs(s, f);
            }
        }
        MirStmt::Match { value, arms } => {
            walk_expr_tree(value, f);
            for arm in arms {
                for s in &arm.body {
                    walk_stmt_exprs(s, f);
                }
            }
        }
        MirStmt::Return(Some(expr)) | MirStmt::Drop(expr) => walk_expr_tree(expr, f),
        MirStmt::Return(None) | MirStmt::Break | MirStmt::Continue => {}
    }
}

fn walk_place_exprs(place: &MirPlace, f: &mut dyn FnMut(&MirExpr)) {
    match place {
        MirPlace::Local(_) => {}
        MirPlace::Field { base, .. } => walk_place_exprs(base, f),
        MirPlace::Index { base, index } => {
            walk_place_exprs(base, f);
            walk_expr_tree(index, f);
        }
    }
}

fn walk_expr_tree(expr: &MirExpr, f: &mut dyn FnMut(&MirExpr)) {
    f(expr);
    match expr {
        MirExpr::Binary { left, right, .. } => {
            walk_expr_tree(left, f);
            walk_expr_tree(right, f);
        }
        MirExpr::Unary { expr, .. } => walk_expr_tree(expr, f),
        MirExpr::Call { args, .. } => {
            for a in args {
                walk_expr_tree(a, f);
            }
        }
        MirExpr::EnumVariant { payload: Some(p), .. } => walk_expr_tree(p, f),
        MirExpr::StructLiteral { fields, .. } => {
            for field in fields {
                walk_expr_tree(&field.value, f);
            }
        }
        MirExpr::ArrayLiteral { elements } | MirExpr::Tuple { elements } => {
            for e in elements {
                walk_expr_tree(e, f);
            }
        }
        MirExpr::FieldAccess { base, .. } => walk_expr_tree(base, f),
        MirExpr::Index { base, index } => {
            walk_expr_tree(base, f);
            walk_expr_tree(index, f);
        }
        MirExpr::Lambda { body, .. } => walk_expr_tree(body, f),
        _ => {}
    }
}
