use crate::ast::{BinaryOp, Expr, Function, Item, Module, Pattern, Stmt, UnaryOp};
use crate::diagnostic::{Diagnostic, Span};
use crate::mir::{self, MirExpr, MirFunction, MirPattern, MirPlace, MirStmt, Program};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::rc::Rc;

// Runaway-loop backstop. Large enough that real workloads (e.g. the M7
// fixpoint: the Zeta-written frontend lexing/parsing its own 7.5k-line source
// inside this interpreter) never trip it, while a genuinely infinite loop
// still aborts instead of hanging forever.
const LOOP_LIMIT: usize = 1_000_000_000;

// `Eq` is intentionally omitted: `Value::Float(f64)` is only `PartialEq`. Float
// equality follows IEEE-754 (NaN != NaN); the corpus never relies on NaN.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    // `fields`/array contents live behind `Rc` so cloning a Value (which the
    // tree-walking evaluator does on every `Load`, field read, and argument
    // pass) is O(1) instead of deep-copying every parallel arena array. Writes
    // go through `Rc::make_mut`, giving copy-on-write: shared values stay
    // immutable (preserving Zeta's value semantics) while the common
    // unique-owner path mutates in place. This is what makes the M7 fixpoint —
    // the Zeta frontend processing its own 7.5k-line source inside this
    // interpreter — finish in seconds instead of tens of CPU-minutes.
    Struct {
        ty: String,
        fields: Rc<BTreeMap<String, Value>>,
    },
    Enum {
        ty: String,
        variant: String,
        payload: Option<Box<Value>>,
    },
    Array(Rc<Vec<Value>>),
    // Tuples are fixed-arity, immutable aggregates; like arrays they share
    // contents behind `Rc` so cloning the Value is O(1).
    Tuple(Rc<Vec<Value>>),
    // A closure: a function value carrying the variables it captured (by value,
    // snapshotted at creation — matching Zeta's value semantics) alongside its
    // parameter names and body. The body is either MIR or AST depending on which
    // interpreter created it; a given run only ever produces one kind.
    Closure(Rc<Closure>),
    Unit,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Closure {
    params: Vec<String>,
    captured: HashMap<String, Value>,
    body: ClosureBody,
}

#[derive(Debug, Clone, PartialEq)]
enum ClosureBody {
    Mir(MirExpr),
    Ast(Expr),
}

/// 赋值左值展平后的一步:字段名或已求值的数组下标。
#[derive(Debug, Clone)]
enum PlaceStep {
    Field(String),
    Index(usize),
}

pub fn run(module: &Module) -> Result<Value, Vec<Diagnostic>> {
    let program = mir::lower(module);
    run_mir(&program)
}

pub fn run_mir(program: &Program) -> Result<Value, Vec<Diagnostic>> {
    let Some(main) = find_mir_main(program) else {
        return Err(vec![runtime_error(
            "RUNTIME_NO_MAIN",
            "expected a `main` function",
        )]);
    };
    if !main.params.is_empty() {
        return Err(vec![runtime_error(
            "RUNTIME_MAIN_PARAMS",
            "`main` must not take parameters for Stage 0 execution",
        )]);
    }
    mir::verify(program)?;

    let mut runtime = MirRuntime::new(program);
    // Run `main` from the runtime's own `Rc<MirFunction>` copy (not the
    // borrowed `program` one) so the `MirExpr` node addresses the interpreter
    // walks match the ones `movable_loads` was computed over.
    let main = runtime
        .functions
        .get("main")
        .cloned()
        .expect("main present after find_mir_main");
    runtime.call_function(&main).map_err(|err| vec![err])
}

/// A persistent MIR runtime supporting state-preserving hot code reload (M-hot
/// slice 1). Unlike `run_mir`, a `HotRuntime` lives across many `call`s: it
/// holds the swappable function table, while the caller threads the program
/// STATE value through `call(..)` and back. `hot_swap` atomically replaces the
/// function table with a newly lowered program — the same accumulated state is
/// then fed to the new code. See docs/compiler/hot-reload-design.md.
pub struct HotRuntime {
    inner: MirRuntime,
}

impl HotRuntime {
    pub fn new(program: &Program) -> Self {
        HotRuntime {
            inner: MirRuntime::new(program),
        }
    }

    /// Call a named function (e.g. `step`) with argument values; returns its
    /// result. The threaded state lives in `args`/return, NOT inside the runtime.
    pub fn call(&mut self, name: &str, args: Vec<Value>) -> Result<Value, Vec<Diagnostic>> {
        self.inner.invoke(name, args).map_err(|err| vec![err])
    }

    /// Hot-swap the running function table to a newly lowered program. Any state
    /// the caller is threading survives untouched.
    pub fn hot_swap(&mut self, program: &Program) {
        self.inner.reload(program);
    }

    /// Whether a function of this name is in the (possibly hot-swapped) table.
    pub fn has(&self, name: &str) -> bool {
        self.inner.functions.contains_key(name)
    }

    /// Names of functions a hot reload to `next` would change but which are NOT
    /// marked `reloadable` — i.e. boundary violations. A function present in both
    /// the running table and `next` whose signature or body differs, and which
    /// `next` does not mark `reloadable`, may not be swapped: in a native/release
    /// build it could be inlined or statically dispatched, so changing it needs a
    /// restart. Returns the offending names (sorted); empty means the swap is
    /// allowed. Enforces the §3 coarse-grained-boundary discipline.
    pub fn non_reloadable_changes(&self, next: &Program) -> Vec<String> {
        let mut changed = Vec::new();
        for new_fn in &next.functions {
            if new_fn.reloadable {
                continue;
            }
            if let Some(current) = self.inner.functions.get(&new_fn.name) {
                if function_semantics_differ(current, new_fn) {
                    changed.push(new_fn.name.clone());
                }
            }
        }
        changed.sort();
        changed
    }
}

/// A long-running hot-reloadable service over the `init` / `step` / `render`
/// convention (hot-reload slice 2; see docs/compiler/hot-reload-design.md):
///
///   fn init() -> State                    // produce the initial state
///   fn step(state: State, input) -> State // advance one tick (hot-swappable)
///   fn render(state: State) -> String     // optional; how to display the state
///
/// `ServiceDriver` holds the live STATE and the swappable runtime. `tick`
/// advances the state; `try_reload` swaps in new code WITHOUT disturbing the
/// accumulated state; and a *failed* reload (e.g. a compile error in the new
/// source) leaves the old code and state running, so the service survives a bad
/// edit. This is the testable core the `zeta serve` CLI shell drives.
pub struct ServiceDriver {
    runtime: HotRuntime,
    state: Value,
}

impl ServiceDriver {
    /// Start a service from source: lower it, then `state = init()`.
    pub fn start(source: &str) -> Result<ServiceDriver, Vec<Diagnostic>> {
        let program = crate::lower_source(source)?;
        let mut runtime = HotRuntime::new(&program);
        let state = runtime.call("init", Vec::new())?;
        Ok(ServiceDriver { runtime, state })
    }

    /// Advance the state by one `step(state, input)`; returns the new state.
    pub fn tick(&mut self, input: Value) -> Result<Value, Vec<Diagnostic>> {
        let current = self.state.clone();
        let next = self.runtime.call("step", vec![current, input])?;
        self.state = next;
        Ok(self.state.clone())
    }

    /// Render the current state: calls `render(state)` if the program defines it,
    /// else falls back to the value's `Display`.
    pub fn render(&mut self) -> Result<String, Vec<Diagnostic>> {
        if self.runtime.has("render") {
            let current = self.state.clone();
            return match self.runtime.call("render", vec![current])? {
                Value::String(text) => Ok(text),
                other => Ok(other.to_string()),
            };
        }
        Ok(self.state.to_string())
    }

    /// Hot-swap to new source. Rejected (leaving old code + state running) when:
    /// (a) the new source fails to compile, or (b) it changes a function that is
    /// not marked `reloadable` (a coarse-grained-boundary violation, §3). Either
    /// way the service survives a bad edit.
    pub fn try_reload(&mut self, source: &str) -> Result<(), Vec<Diagnostic>> {
        let program = crate::lower_source(source)?;
        let changed = self.runtime.non_reloadable_changes(&program);
        if !changed.is_empty() {
            return Err(vec![runtime_error(
                "HOT_RELOAD_NON_RELOADABLE",
                format!(
                    "cannot hot-swap: non-`reloadable` function(s) changed: {} \
                     — mark them `reloadable fn` (accepting the boundary) or restart",
                    changed.join(", ")
                ),
            )]);
        }
        self.runtime.hot_swap(&program);
        Ok(())
    }

    pub fn state(&self) -> &Value {
        &self.state
    }
}

#[derive(Debug, Default)]
pub struct ReplSession {
    locals: HashMap<String, Value>,
    functions: HashMap<String, Function>,
    enum_variants: HashMap<String, HashMap<String, Option<String>>>,
}

impl ReplSession {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn eval_module(&mut self, module: &Module) -> Result<Value, Vec<Diagnostic>> {
        for item in &module.items {
            if let Item::Function(function) = item {
                self.functions
                    .insert(function.name.clone(), function.clone());
            }
            if let Item::Enum(decl) = item {
                self.enum_variants.insert(
                    decl.name.clone(),
                    decl.variants
                        .iter()
                        .map(|variant| (variant.name.clone(), variant.payload_type.clone()))
                        .collect(),
                );
            }
        }

        let Some(main) = find_main(module) else {
            return Ok(Value::Unit);
        };
        if !main.params.is_empty() {
            return Err(vec![runtime_error(
                "RUNTIME_MAIN_PARAMS",
                "`main` must not take parameters for Stage 0 REPL execution",
            )]);
        }

        let mut runtime = Runtime::from_parts(self.functions.clone(), self.enum_variants.clone());
        match runtime.eval_stmts(&main.body, &mut self.locals) {
            Ok(Control::Return(value)) => Ok(value),
            Ok(Control::Continue) => Ok(Value::Unit),
            Ok(Control::BreakLoop) => Err(vec![runtime_error(
                "RUNTIME_BREAK_OUTSIDE_LOOP",
                "`break` reached function boundary",
            )]),
            Ok(Control::ContinueLoop) => Err(vec![runtime_error(
                "RUNTIME_CONTINUE_OUTSIDE_LOOP",
                "`continue` reached function boundary",
            )]),
            Err(err) => Err(vec![err]),
        }
    }
}

fn find_main(module: &Module) -> Option<&Function> {
    module.items.iter().find_map(|item| match item {
        Item::Function(function) if function.name == "main" => Some(function),
        _ => None,
    })
}

fn find_mir_main(program: &Program) -> Option<&MirFunction> {
    program
        .functions
        .iter()
        .find(|function| function.name == "main")
}

// ---------------------------------------------------------------------------
// Last-use (liveness) analysis for move-on-load.
//
// A backward liveness pass over each function's structured MIR. It marks every
// `Load(name)` site that is the LAST use of `name` (i.e. `name` is dead right
// after the load): on every forward path the next event touching `name` is
// either a full rebind or function exit, never another read. The interpreter
// then MOVES such loads out of `locals` (refcount drops to 1) instead of
// cloning, so copy-on-write writes mutate in place — O(n) instead of O(n^2).
//
// Soundness: an over-approximation of liveness is always safe (it only
// SUPPRESSES moves). `while` is solved to a fixpoint so loop-carried liveness
// is captured; `for-*` loops (which the self-hosting frontend never uses, but
// the run corpus does) are handled conservatively — every name they mention is
// kept live and nothing inside is marked movable. Any residual unsoundness would
// surface immediately as a divergence in the run-corpus / fixpoint oracle gates.
type LiveSet = HashSet<String>;

struct LoopCtx {
    brk: LiveSet,
    cont: LiveSet,
}

fn compute_movable_loads(functions: &HashMap<String, Rc<MirFunction>>) -> HashSet<*const MirExpr> {
    let mut movable = HashSet::new();
    for function in functions.values() {
        // live-out of a function body is empty: nothing is live after return.
        live_stmts(&function.body, LiveSet::new(), None, true, &mut movable);
    }
    movable
}

fn live_stmts(
    stmts: &[MirStmt],
    mut live: LiveSet,
    ctx: Option<&LoopCtx>,
    mark: bool,
    movable: &mut HashSet<*const MirExpr>,
) -> LiveSet {
    for stmt in stmts.iter().rev() {
        live = live_stmt(stmt, live, ctx, mark, movable);
    }
    live
}

fn live_stmt(
    stmt: &MirStmt,
    mut live: LiveSet,
    ctx: Option<&LoopCtx>,
    mark: bool,
    movable: &mut HashSet<*const MirExpr>,
) -> LiveSet {
    match stmt {
        MirStmt::Local { name, value, .. } => {
            // Binding `name` kills its old value before the RHS is evaluated, so
            // the RHS's final read of `name` (if any) is a last use.
            live.remove(name);
            live_expr(value, live, mark, movable)
        }
        MirStmt::Store { place, value } => match place {
            MirPlace::Local(name) => {
                // Full reassignment: kills the old binding, like `Local`.
                live.remove(name);
                live_expr(value, live, mark, movable)
            }
            _ => {
                // Field/index store is a read-modify-write of the root: keep all
                // names the place mentions live, and never move them.
                live = live_place_uses(place, live);
                live_expr(value, live, mark, movable)
            }
        },
        MirStmt::If {
            condition,
            then_body,
            else_body,
        } => {
            let lt = live_stmts(then_body, live.clone(), ctx, mark, movable);
            let le = live_stmts(else_body, live, ctx, mark, movable);
            let mut merged = lt;
            merged.extend(le);
            live_expr(condition, merged, mark, movable)
        }
        MirStmt::While { condition, body } => live_while(condition, body, live, mark, movable),
        MirStmt::Match { value, arms } => {
            let mut merged = LiveSet::new();
            for arm in arms {
                let mut al = live_stmts(&arm.body, live.clone(), ctx, mark, movable);
                for binding in liveness_pattern_bindings(&arm.pattern) {
                    al.remove(&binding);
                }
                merged.extend(al);
            }
            live_expr(value, merged, mark, movable)
        }
        MirStmt::Return(Some(expr)) => live_expr(expr, LiveSet::new(), mark, movable),
        MirStmt::Return(None) => LiveSet::new(),
        MirStmt::Break => ctx.map(|c| c.brk.clone()).unwrap_or_default(),
        MirStmt::Continue => ctx.map(|c| c.cont.clone()).unwrap_or_default(),
        MirStmt::Drop(expr) => live_expr(expr, live, mark, movable),
        // Conservative for-loops: keep every mentioned name live, mark nothing.
        MirStmt::ForIn { .. } | MirStmt::ForRange { .. } | MirStmt::ForC { .. } => {
            collect_stmt_names(stmt, &mut live);
            live
        }
    }
}

/// Solve `while` to a fixpoint so loop-carried liveness (a name read on the next
/// iteration) is captured, then do one marking pass with the converged sets.
fn live_while(
    condition: &MirExpr,
    body: &[MirStmt],
    live_out: LiveSet,
    mark: bool,
    movable: &mut HashSet<*const MirExpr>,
) -> LiveSet {
    let brk = live_out.clone();
    // `cont` = live-in of the condition = the loop's live-in. Grows monotonically.
    let mut cont = live_out;
    loop {
        let ctx = LoopCtx {
            brk: brk.clone(),
            cont: cont.clone(),
        };
        // body live-out flows to the condition (= cont); plus the exit edge (brk).
        let lib = live_stmts(body, cont.clone(), Some(&ctx), false, movable);
        let mut cond_out = lib;
        cond_out.extend(brk.iter().cloned());
        let next = live_expr(condition, cond_out, false, movable);
        if next == cont {
            break;
        }
        cont = next;
    }
    if mark {
        let ctx = LoopCtx {
            brk: brk.clone(),
            cont: cont.clone(),
        };
        let lib = live_stmts(body, cont.clone(), Some(&ctx), true, movable);
        let mut cond_out = lib;
        cond_out.extend(brk.iter().cloned());
        live_expr(condition, cond_out, true, movable);
    }
    cont
}

/// Process an expression backward (reverse evaluation order), marking last-use
/// loads. Returns the live set BEFORE the expression.
fn live_expr(
    expr: &MirExpr,
    mut live: LiveSet,
    mark: bool,
    movable: &mut HashSet<*const MirExpr>,
) -> LiveSet {
    match expr {
        MirExpr::Load(name) => {
            if mark && !live.contains(name) {
                movable.insert(expr as *const MirExpr);
            }
            live.insert(name.clone());
            live
        }
        MirExpr::Int(_) | MirExpr::Float(_) | MirExpr::String(_) | MirExpr::Bool(_) => live,
        MirExpr::Binary { left, right, .. } => {
            // Eval order is left then right; process right first so left sees
            // right's uses. (`&&`/`||` may skip the right at runtime — treating
            // it as always-used only over-approximates liveness, which is safe.)
            let live = live_expr(right, live, mark, movable);
            live_expr(left, live, mark, movable)
        }
        MirExpr::Unary { expr, .. } => live_expr(expr, live, mark, movable),
        MirExpr::Call { args, .. } => {
            for arg in args.iter().rev() {
                live = live_expr(arg, live, mark, movable);
            }
            live
        }
        MirExpr::EnumVariant { payload, .. } => match payload {
            Some(payload) => live_expr(payload, live, mark, movable),
            None => live,
        },
        MirExpr::StructLiteral { fields, .. } => {
            for field in fields.iter().rev() {
                live = live_expr(&field.value, live, mark, movable);
            }
            live
        }
        MirExpr::FieldAccess { base, .. } => live_expr(base, live, mark, movable),
        MirExpr::ArrayLiteral { elements } | MirExpr::Tuple { elements } => {
            for element in elements.iter().rev() {
                live = live_expr(element, live, mark, movable);
            }
            live
        }
        MirExpr::Lambda { body, .. } => {
            // The closure captures its free variables by value at creation time,
            // so every name the body references is used here. Keep them live (and
            // never movable: the capture clones them, leaving the originals).
            let mut names = LiveSet::new();
            collect_expr_names(body, &mut names);
            for name in names {
                live.insert(name);
            }
            live
        }
        MirExpr::Index { base, index } => {
            // Eval order base then index; process index first.
            let live = live_expr(index, live, mark, movable);
            live_expr(base, live, mark, movable)
        }
    }
}

fn liveness_pattern_bindings(pattern: &MirPattern) -> Vec<String> {
    match pattern {
        MirPattern::Name(name) => vec![name.clone()],
        MirPattern::Variant {
            binding: Some(binding),
            ..
        } => vec![binding.clone()],
        _ => Vec::new(),
    }
}

/// Names a place reads (its root, plus any index expressions). Used to keep them
/// live across a read-modify-write store without marking them movable.
fn live_place_uses(place: &MirPlace, mut live: LiveSet) -> LiveSet {
    match place {
        MirPlace::Local(name) => {
            live.insert(name.clone());
            live
        }
        MirPlace::Field { base, .. } => live_place_uses(base, live),
        MirPlace::Index { base, index } => {
            collect_expr_names(index, &mut live);
            live_place_uses(base, live)
        }
    }
}

fn collect_expr_names(expr: &MirExpr, out: &mut LiveSet) {
    match expr {
        MirExpr::Load(name) => {
            out.insert(name.clone());
        }
        MirExpr::Int(_) | MirExpr::Float(_) | MirExpr::String(_) | MirExpr::Bool(_) => {}
        MirExpr::Binary { left, right, .. } => {
            collect_expr_names(left, out);
            collect_expr_names(right, out);
        }
        MirExpr::Unary { expr, .. } => collect_expr_names(expr, out),
        MirExpr::Call { args, .. } => {
            for arg in args {
                collect_expr_names(arg, out);
            }
        }
        MirExpr::EnumVariant { payload, .. } => {
            if let Some(payload) = payload {
                collect_expr_names(payload, out);
            }
        }
        MirExpr::StructLiteral { fields, .. } => {
            for field in fields {
                collect_expr_names(&field.value, out);
            }
        }
        MirExpr::FieldAccess { base, .. } => collect_expr_names(base, out),
        MirExpr::ArrayLiteral { elements } | MirExpr::Tuple { elements } => {
            for element in elements {
                collect_expr_names(element, out);
            }
        }
        MirExpr::Lambda { body, .. } => collect_expr_names(body, out),
        MirExpr::Index { base, index } => {
            collect_expr_names(base, out);
            collect_expr_names(index, out);
        }
    }
}

fn collect_place_names(place: &MirPlace, out: &mut LiveSet) {
    match place {
        MirPlace::Local(name) => {
            out.insert(name.clone());
        }
        MirPlace::Field { base, .. } => collect_place_names(base, out),
        MirPlace::Index { base, index } => {
            collect_expr_names(index, out);
            collect_place_names(base, out);
        }
    }
}

fn collect_stmt_names(stmt: &MirStmt, out: &mut LiveSet) {
    match stmt {
        MirStmt::Local { value, .. } => collect_expr_names(value, out),
        MirStmt::Store { place, value } => {
            collect_place_names(place, out);
            collect_expr_names(value, out);
        }
        MirStmt::If {
            condition,
            then_body,
            else_body,
        } => {
            collect_expr_names(condition, out);
            for stmt in then_body.iter().chain(else_body) {
                collect_stmt_names(stmt, out);
            }
        }
        MirStmt::While { condition, body } => {
            collect_expr_names(condition, out);
            for stmt in body {
                collect_stmt_names(stmt, out);
            }
        }
        MirStmt::ForIn {
            iterable, body, ..
        } => {
            collect_expr_names(iterable, out);
            for stmt in body {
                collect_stmt_names(stmt, out);
            }
        }
        MirStmt::ForRange {
            start, end, body, ..
        } => {
            collect_expr_names(start, out);
            collect_expr_names(end, out);
            for stmt in body {
                collect_stmt_names(stmt, out);
            }
        }
        MirStmt::ForC {
            init,
            condition,
            step,
            body,
        } => {
            collect_stmt_names(init, out);
            collect_expr_names(condition, out);
            collect_stmt_names(step, out);
            for stmt in body {
                collect_stmt_names(stmt, out);
            }
        }
        MirStmt::Match { value, arms } => {
            collect_expr_names(value, out);
            for arm in arms {
                for stmt in &arm.body {
                    collect_stmt_names(stmt, out);
                }
            }
        }
        MirStmt::Return(Some(expr)) | MirStmt::Drop(expr) => collect_expr_names(expr, out),
        MirStmt::Return(None) | MirStmt::Break | MirStmt::Continue => {}
    }
}

/// Whether two MIR functions differ in a way that matters for hot-swap safety:
/// signature (param names+types, return type) or body. Param SPANS are ignored
/// on purpose — a function's byte offsets shift whenever earlier code is edited,
/// so comparing spans would flag unchanged functions as changed.
fn function_semantics_differ(a: &MirFunction, b: &MirFunction) -> bool {
    a.return_type != b.return_type
        || a.body != b.body
        || a.params.len() != b.params.len()
        || a.params
            .iter()
            .zip(&b.params)
            .any(|(x, y)| x.name != y.name || x.ty != y.ty)
}

struct MirRuntime {
    functions: HashMap<String, Rc<MirFunction>>,
    enum_variants: HashMap<String, HashMap<String, Option<String>>>,
    loop_steps: usize,
    // `Load` sites (keyed by `MirExpr` node address) whose value is dead
    // immediately afterwards — the interpreter MOVES (removes) these out of
    // `locals` instead of cloning, so the copy-on-write `Value`'s `Rc` reaches
    // refcount 1 and writes mutate in place. This is what turns the residual
    // O(n^2) arena-threading (`let r = parse_x(a, ..); a = r.arena`) into O(n).
    // Computed once over the `Rc<MirFunction>` bodies, whose node addresses stay
    // stable for the runtime's lifetime (see `compute_movable_loads`).
    movable_loads: HashSet<*const MirExpr>,
}

impl MirRuntime {
    fn new(program: &Program) -> Self {
        let functions: HashMap<String, Rc<MirFunction>> = program
            .functions
            .iter()
            .map(|function| (function.name.clone(), Rc::new(function.clone())))
            .collect();
        let movable_loads = compute_movable_loads(&functions);
        Self {
            functions,
            movable_loads,
            enum_variants: program
                .enums
                .iter()
                .map(|enum_decl| (enum_decl.name.clone(), enum_decl.variants.clone()))
                .map(|(name, variants)| {
                    (
                        name,
                        variants
                            .into_iter()
                            .map(|variant| (variant.name, variant.payload_type))
                            .collect(),
                    )
                })
                .collect(),
            loop_steps: 0,
        }
    }

    fn call_function(&mut self, function: &MirFunction) -> Result<Value, Diagnostic> {
        let mut locals = HashMap::new();
        match self.eval_stmts(&function.body, &mut locals)? {
            Control::Return(value) => Ok(value),
            Control::Continue => Ok(Value::Unit),
            Control::BreakLoop => Err(runtime_error(
                "RUNTIME_BREAK_OUTSIDE_LOOP",
                "`break` reached function boundary",
            )),
            Control::ContinueLoop => Err(runtime_error(
                "RUNTIME_CONTINUE_OUTSIDE_LOOP",
                "`continue` reached function boundary",
            )),
        }
    }

    /// Invoke a named function with pre-evaluated argument values. Used by the
    /// hot-reload service loop to call `step(state, input)` across iterations:
    /// the STATE lives in the caller (threaded value), the swappable code lives
    /// in `self.functions`. Each invocation gets a fresh loop-step budget so a
    /// long-running service is not capped by a single accumulating counter.
    fn invoke(&mut self, name: &str, args: Vec<Value>) -> Result<Value, Diagnostic> {
        let Some(function) = self.functions.get(name).cloned() else {
            return Err(runtime_error(
                "RUNTIME_UNKNOWN_FUNCTION",
                format!("unknown function `{name}`"),
            ));
        };
        if function.params.len() != args.len() {
            return Err(runtime_error(
                "RUNTIME_CALL_ARITY",
                format!(
                    "function `{name}` expects {} arguments, found {}",
                    function.params.len(),
                    args.len()
                ),
            ));
        }
        self.loop_steps = 0;
        let mut locals = HashMap::new();
        for (param, arg) in function.params.iter().zip(args) {
            locals.insert(param.name.clone(), arg);
        }
        match self.eval_stmts(&function.body, &mut locals)? {
            Control::Return(value) => Ok(value),
            Control::Continue => Ok(Value::Unit),
            Control::BreakLoop => Err(runtime_error(
                "RUNTIME_BREAK_OUTSIDE_LOOP",
                "`break` reached function boundary",
            )),
            Control::ContinueLoop => Err(runtime_error(
                "RUNTIME_CONTINUE_OUTSIDE_LOOP",
                "`continue` reached function boundary",
            )),
        }
    }

    /// Atomically swap the function table (and its derived liveness) to a newly
    /// lowered program — the hot-code-reload core. The function table is already
    /// `HashMap<String, Rc<MirFunction>>`, so replacing an entry is cheap; the
    /// caller's threaded STATE value is untouched, so it survives the swap.
    fn reload(&mut self, program: &Program) {
        self.functions = program
            .functions
            .iter()
            .map(|function| (function.name.clone(), Rc::new(function.clone())))
            .collect();
        // `movable_loads` keys on `MirExpr` node addresses, which the swap
        // invalidated — recompute over the new bodies.
        self.movable_loads = compute_movable_loads(&self.functions);
        self.enum_variants = program
            .enums
            .iter()
            .map(|enum_decl| {
                (
                    enum_decl.name.clone(),
                    enum_decl
                        .variants
                        .iter()
                        .map(|variant| (variant.name.clone(), variant.payload_type.clone()))
                        .collect(),
                )
            })
            .collect();
    }

    fn eval_stmts(
        &mut self,
        stmts: &[MirStmt],
        locals: &mut HashMap<String, Value>,
    ) -> Result<Control, Diagnostic> {
        for stmt in stmts {
            match self.eval_stmt(stmt, locals)? {
                Control::Continue => {}
                control @ (Control::Return(_) | Control::BreakLoop | Control::ContinueLoop) => {
                    return Ok(control);
                }
            }
        }
        Ok(Control::Continue)
    }

    fn eval_stmt(
        &mut self,
        stmt: &MirStmt,
        locals: &mut HashMap<String, Value>,
    ) -> Result<Control, Diagnostic> {
        match stmt {
            MirStmt::Local { name, value, .. } => {
                let value = self.eval_expr(value, locals)?;
                locals.insert(name.clone(), value);
                Ok(Control::Continue)
            }
            MirStmt::Store { place, value } => {
                let value = self.eval_expr(value, locals)?;
                self.store_place(place, value, locals)?;
                Ok(Control::Continue)
            }
            MirStmt::If {
                condition,
                then_body,
                else_body,
            } => {
                let condition = self.eval_expr(condition, locals)?;
                let Value::Bool(condition) = condition else {
                    return Err(runtime_error(
                        "RUNTIME_IF_CONDITION",
                        "if condition must evaluate to Bool",
                    ));
                };
                if condition {
                    self.eval_stmts(then_body, locals)
                } else {
                    self.eval_stmts(else_body, locals)
                }
            }
            MirStmt::While { condition, body } => {
                loop {
                    self.loop_steps += 1;
                    if self.loop_steps > LOOP_LIMIT {
                        return Err(runtime_error(
                            "RUNTIME_LOOP_LIMIT",
                            "loop exceeded the Stage 0 execution step limit",
                        ));
                    }
                    let condition = self.eval_expr(condition, locals)?;
                    let Value::Bool(condition) = condition else {
                        return Err(runtime_error(
                            "RUNTIME_WHILE_CONDITION",
                            "while condition must evaluate to Bool",
                        ));
                    };
                    if !condition {
                        break;
                    }
                    match self.eval_stmts(body, locals)? {
                        Control::Continue => {}
                        Control::BreakLoop => break,
                        Control::ContinueLoop => continue,
                        returned @ Control::Return(_) => return Ok(returned),
                    }
                }
                Ok(Control::Continue)
            }
            MirStmt::ForIn {
                binding,
                iterable,
                body,
            } => {
                let iterable = self.eval_expr(iterable, locals)?;
                let Value::Array(elements) = iterable else {
                    return Err(runtime_error(
                        "RUNTIME_FOR_ITERABLE",
                        "for-in iterable must evaluate to an array",
                    ));
                };
                let saved = locals.remove(binding);
                let mut control = Control::Continue;
                for element in elements.iter() {
                    self.loop_steps += 1;
                    if self.loop_steps > LOOP_LIMIT {
                        if let Some(saved) = saved {
                            locals.insert(binding.clone(), saved);
                        } else {
                            locals.remove(binding);
                        }
                        return Err(runtime_error(
                            "RUNTIME_LOOP_LIMIT",
                            "loop exceeded the Stage 0 execution step limit",
                        ));
                    }
                    locals.insert(binding.clone(), element.clone());
                    match self.eval_stmts(body, locals)? {
                        Control::Continue => {}
                        Control::BreakLoop => break,
                        Control::ContinueLoop => continue,
                        returned @ Control::Return(_) => {
                            control = returned;
                            break;
                        }
                    }
                }
                if let Some(saved) = saved {
                    locals.insert(binding.clone(), saved);
                } else {
                    locals.remove(binding);
                }
                Ok(control)
            }
            MirStmt::ForRange {
                binding,
                start,
                end,
                body,
            } => {
                let start_value = self.eval_expr(start, locals)?;
                let end_value = self.eval_expr(end, locals)?;
                let (Value::Int(start_value), Value::Int(end_value)) = (start_value, end_value)
                else {
                    return Err(runtime_error(
                        "RUNTIME_FOR_RANGE_BOUND",
                        "for-in range bounds must evaluate to Int",
                    ));
                };
                let saved = locals.remove(binding);
                let mut control = Control::Continue;
                let mut i = start_value;
                while i < end_value {
                    self.loop_steps += 1;
                    if self.loop_steps > LOOP_LIMIT {
                        if let Some(saved) = saved {
                            locals.insert(binding.clone(), saved);
                        } else {
                            locals.remove(binding);
                        }
                        return Err(runtime_error(
                            "RUNTIME_LOOP_LIMIT",
                            "loop exceeded the Stage 0 execution step limit",
                        ));
                    }
                    locals.insert(binding.clone(), Value::Int(i));
                    match self.eval_stmts(body, locals)? {
                        Control::Continue => {}
                        Control::BreakLoop => break,
                        Control::ContinueLoop => {
                            i += 1;
                            continue;
                        }
                        returned @ Control::Return(_) => {
                            control = returned;
                            break;
                        }
                    }
                    i += 1;
                }
                if let Some(saved) = saved {
                    locals.insert(binding.clone(), saved);
                } else {
                    locals.remove(binding);
                }
                Ok(control)
            }
            MirStmt::ForC {
                init,
                condition,
                step,
                body,
            } => {
                // init runs once; its binding stays in locals (typecheck guarantees
                // it isn't referenced outside the for).
                match self.eval_stmt(init, locals)? {
                    Control::Continue => {}
                    other => return Ok(other),
                }
                loop {
                    self.loop_steps += 1;
                    if self.loop_steps > LOOP_LIMIT {
                        return Err(runtime_error(
                            "RUNTIME_LOOP_LIMIT",
                            "loop exceeded the Stage 0 execution step limit",
                        ));
                    }
                    let condition = self.eval_expr(condition, locals)?;
                    let Value::Bool(condition) = condition else {
                        return Err(runtime_error(
                            "RUNTIME_FORC_CONDITION",
                            "for condition must evaluate to Bool",
                        ));
                    };
                    if !condition {
                        break;
                    }
                    match self.eval_stmts(body, locals)? {
                        Control::Continue => {}
                        Control::BreakLoop => break,
                        Control::ContinueLoop => {}
                        returned @ Control::Return(_) => return Ok(returned),
                    }
                    match self.eval_stmt(step, locals)? {
                        Control::Continue => {}
                        returned @ Control::Return(_) => return Ok(returned),
                        Control::BreakLoop | Control::ContinueLoop => {}
                    }
                }
                Ok(Control::Continue)
            }
            MirStmt::Match { value, arms } => {
                let value = self.eval_expr(value, locals)?;
                for arm in arms {
                    if let Some(bindings) = mir_pattern_bindings(&arm.pattern, &value)? {
                        let saved = apply_bindings(locals, bindings);
                        let result = self.eval_stmts(&arm.body, locals);
                        restore_bindings(locals, saved);
                        return result;
                    }
                }
                Err(runtime_error(
                    "RUNTIME_MATCH_NON_EXHAUSTIVE",
                    "match did not select an arm",
                ))
            }
            MirStmt::Return(Some(value)) => Ok(Control::Return(self.eval_expr(value, locals)?)),
            MirStmt::Return(None) => Ok(Control::Return(Value::Unit)),
            MirStmt::Break => Ok(Control::BreakLoop),
            MirStmt::Continue => Ok(Control::ContinueLoop),
            MirStmt::Drop(value) => {
                let _ = self.eval_expr(value, locals)?;
                Ok(Control::Continue)
            }
        }
    }

    fn store_place(
        &mut self,
        place: &MirPlace,
        value: Value,
        locals: &mut HashMap<String, Value>,
    ) -> Result<(), Diagnostic> {
        let (root, path) = self.flatten_place(place, locals)?;
        write_through_path(locals, &root, &path, value)
    }

    fn flatten_place(
        &mut self,
        place: &MirPlace,
        locals: &mut HashMap<String, Value>,
    ) -> Result<(String, Vec<PlaceStep>), Diagnostic> {
        match place {
            MirPlace::Local(name) => Ok((name.clone(), Vec::new())),
            MirPlace::Field { base, field } => {
                let (root, mut path) = self.flatten_place(base, locals)?;
                path.push(PlaceStep::Field(field.clone()));
                Ok((root, path))
            }
            MirPlace::Index { base, index } => {
                let (root, mut path) = self.flatten_place(base, locals)?;
                let idx = self.eval_expr(index, locals)?;
                let Value::Int(i) = idx else {
                    return Err(runtime_error(
                        "RUNTIME_ASSIGN_INDEX_TYPE",
                        "assignment index must evaluate to Int",
                    ));
                };
                if i < 0 {
                    return Err(runtime_error(
                        "RUNTIME_ASSIGN_INDEX_BOUNDS",
                        "negative assignment index",
                    ));
                }
                path.push(PlaceStep::Index(i as usize));
                Ok((root, path))
            }
        }
    }

    /// Apply a closure: bind args over the captured environment and evaluate the
    /// (expression) body. The body is MIR because a MIR run only mints MIR-bodied
    /// closures.
    fn apply_closure(&mut self, closure: &Closure, args: Vec<Value>) -> Result<Value, Diagnostic> {
        if closure.params.len() != args.len() {
            return Err(runtime_error(
                "RUNTIME_CALL_ARITY",
                format!(
                    "closure expects {} arguments, found {}",
                    closure.params.len(),
                    args.len()
                ),
            ));
        }
        let ClosureBody::Mir(body) = &closure.body else {
            return Err(runtime_error(
                "RUNTIME_CLOSURE_BODY",
                "closure body is not MIR in the MIR interpreter",
            ));
        };
        let mut call_locals = closure.captured.clone();
        for (name, value) in closure.params.iter().zip(args) {
            call_locals.insert(name.clone(), value);
        }
        self.eval_expr(body, &mut call_locals)
    }

    fn eval_expr(
        &mut self,
        expr: &MirExpr,
        locals: &mut HashMap<String, Value>,
    ) -> Result<Value, Diagnostic> {
        match expr {
            MirExpr::Load(name) => {
                // If this load is `name`'s last use, MOVE it out (refcount 1 →
                // in-place writes); otherwise clone, leaving the binding intact.
                let value = if self.movable_loads.contains(&(expr as *const MirExpr)) {
                    locals.remove(name)
                } else {
                    locals.get(name).cloned()
                };
                value.ok_or_else(|| {
                    runtime_error("RUNTIME_UNKNOWN_NAME", format!("unknown name `{name}`"))
                })
            }
            MirExpr::Int(value) => value.parse::<i64>().map(Value::Int).map_err(|_| {
                runtime_error(
                    "RUNTIME_INT_PARSE",
                    format!("invalid Int literal `{value}`"),
                )
            }),
            MirExpr::Float(value) => value.parse::<f64>().map(Value::Float).map_err(|_| {
                runtime_error(
                    "RUNTIME_FLOAT_PARSE",
                    format!("invalid Float literal `{value}`"),
                )
            }),
            MirExpr::String(value) => Ok(Value::String(value.clone())),
            MirExpr::Bool(value) => Ok(Value::Bool(*value)),
            MirExpr::Binary { op, left, right } => self.eval_binary_expr(*op, left, right, locals),
            MirExpr::Unary { op, expr } => {
                let value = self.eval_expr(expr, locals)?;
                eval_unary(*op, value)
            }
            MirExpr::Call { callee, args } => {
                if is_std_builtin(callee) {
                    let mut arg_values = Vec::with_capacity(args.len());
                    for arg in args {
                        arg_values.push(self.eval_expr(arg, locals)?);
                    }
                    return eval_std_builtin(callee, arg_values);
                }
                let Some(function) = self.functions.get(callee).cloned() else {
                    // Indirect call: `callee` may name a local closure value.
                    if let Some(Value::Closure(closure)) = locals.get(callee).cloned() {
                        let mut arg_values = Vec::with_capacity(args.len());
                        for arg in args {
                            arg_values.push(self.eval_expr(arg, locals)?);
                        }
                        return self.apply_closure(&closure, arg_values);
                    }
                    return Err(runtime_error(
                        "RUNTIME_UNKNOWN_FUNCTION",
                        format!("unknown function `{callee}`"),
                    ));
                };
                if function.params.len() != args.len() {
                    return Err(runtime_error(
                        "RUNTIME_CALL_ARITY",
                        format!(
                            "function `{callee}` expects {} arguments, found {}",
                            function.params.len(),
                            args.len()
                        ),
                    ));
                }
                let mut call_locals = HashMap::new();
                for (param, arg) in function.params.iter().zip(args) {
                    call_locals.insert(param.name.clone(), self.eval_expr(arg, locals)?);
                }
                match self.eval_stmts(&function.body, &mut call_locals)? {
                    Control::Return(value) => Ok(value),
                    Control::Continue => Ok(Value::Unit),
                    Control::BreakLoop => Err(runtime_error(
                        "RUNTIME_BREAK_OUTSIDE_LOOP",
                        "`break` reached function boundary",
                    )),
                    Control::ContinueLoop => Err(runtime_error(
                        "RUNTIME_CONTINUE_OUTSIDE_LOOP",
                        "`continue` reached function boundary",
                    )),
                }
            }
            MirExpr::EnumVariant {
                enum_name,
                variant,
                payload,
            } => {
                let payload = match payload {
                    Some(payload) => Some(Box::new(self.eval_expr(payload, locals)?)),
                    None => None,
                };
                Ok(Value::Enum {
                    ty: enum_name.clone(),
                    variant: variant.clone(),
                    payload,
                })
            }
            MirExpr::StructLiteral { ty, fields } => {
                let mut values = BTreeMap::new();
                for field in fields {
                    values.insert(field.name.clone(), self.eval_expr(&field.value, locals)?);
                }
                Ok(Value::Struct {
                    ty: ty.clone(),
                    fields: Rc::new(values),
                })
            }
            MirExpr::FieldAccess { base, field } => {
                if let MirExpr::Load(enum_name) = base.as_ref() {
                    if let Some(variants) = self.enum_variants.get(enum_name) {
                        if variants.contains_key(field) {
                            return Ok(Value::Enum {
                                ty: enum_name.clone(),
                                variant: field.clone(),
                                payload: None,
                            });
                        }
                        return Err(runtime_error(
                            "RUNTIME_UNKNOWN_VARIANT",
                            format!("unknown variant `{field}` on enum `{enum_name}`"),
                        ));
                    }
                }
                let value = self.eval_expr(base, locals)?;
                if let Value::Array(values) = &value {
                    if field == "len" {
                        return Ok(Value::Int(values.len() as i64));
                    }
                    return Err(runtime_error(
                        "RUNTIME_ARRAY_FIELD",
                        format!("unknown field `{field}` on array; only `len` is supported"),
                    ));
                }
                if let Value::Tuple(values) = &value {
                    return tuple_field(values, field);
                }
                let Value::Struct { ty, fields } = value else {
                    return Err(runtime_error(
                        "RUNTIME_FIELD_BASE",
                        "field access requires a struct value",
                    ));
                };
                fields.get(field).cloned().ok_or_else(|| {
                    runtime_error(
                        "RUNTIME_UNKNOWN_FIELD",
                        format!("unknown field `{field}` on struct `{ty}`"),
                    )
                })
            }
            MirExpr::ArrayLiteral { elements } => {
                let mut values = Vec::with_capacity(elements.len());
                for element in elements {
                    values.push(self.eval_expr(element, locals)?);
                }
                Ok(Value::Array(Rc::new(values)))
            }
            MirExpr::Tuple { elements } => {
                let mut values = Vec::with_capacity(elements.len());
                for element in elements {
                    values.push(self.eval_expr(element, locals)?);
                }
                Ok(Value::Tuple(Rc::new(values)))
            }
            MirExpr::Lambda { params, body } => {
                // Capture the current environment by value (cloning is O(1) via
                // Rc on the heavy payloads). The snapshot freezes the captured
                // variables' values at creation time — Zeta's value semantics.
                Ok(Value::Closure(Rc::new(Closure {
                    params: params.iter().map(|p| p.name.clone()).collect(),
                    captured: locals.clone(),
                    body: ClosureBody::Mir((**body).clone()),
                })))
            }
            MirExpr::Index { base, index } => {
                let base = self.eval_expr(base, locals)?;
                let index = self.eval_expr(index, locals)?;
                index_array_value(base, index)
            }
        }
    }

    fn eval_binary_expr(
        &mut self,
        op: BinaryOp,
        left: &MirExpr,
        right: &MirExpr,
        locals: &mut HashMap<String, Value>,
    ) -> Result<Value, Diagnostic> {
        match op {
            BinaryOp::And => {
                let left = expect_bool(self.eval_expr(left, locals)?, "RUNTIME_LOGICAL_OPERAND")?;
                if !left {
                    return Ok(Value::Bool(false));
                }
                Ok(Value::Bool(expect_bool(
                    self.eval_expr(right, locals)?,
                    "RUNTIME_LOGICAL_OPERAND",
                )?))
            }
            BinaryOp::Or => {
                let left = expect_bool(self.eval_expr(left, locals)?, "RUNTIME_LOGICAL_OPERAND")?;
                if left {
                    return Ok(Value::Bool(true));
                }
                Ok(Value::Bool(expect_bool(
                    self.eval_expr(right, locals)?,
                    "RUNTIME_LOGICAL_OPERAND",
                )?))
            }
            _ => {
                let left = self.eval_expr(left, locals)?;
                let right = self.eval_expr(right, locals)?;
                eval_binary(op, left, right)
            }
        }
    }
}

struct Runtime {
    functions: HashMap<String, Function>,
    enum_variants: HashMap<String, HashMap<String, Option<String>>>,
    loop_steps: usize,
}

impl Runtime {
    fn from_parts(
        functions: HashMap<String, Function>,
        enum_variants: HashMap<String, HashMap<String, Option<String>>>,
    ) -> Self {
        Self {
            functions,
            enum_variants,
            loop_steps: 0,
        }
    }

    fn eval_stmts(
        &mut self,
        stmts: &[Stmt],
        locals: &mut HashMap<String, Value>,
    ) -> Result<Control, Diagnostic> {
        for stmt in stmts {
            match self.eval_stmt(stmt, locals)? {
                Control::Continue => {}
                control @ (Control::Return(_) | Control::BreakLoop | Control::ContinueLoop) => {
                    return Ok(control);
                }
            }
        }
        Ok(Control::Continue)
    }

    fn eval_stmt(
        &mut self,
        stmt: &Stmt,
        locals: &mut HashMap<String, Value>,
    ) -> Result<Control, Diagnostic> {
        match stmt {
            Stmt::Let { name, value, .. } => {
                let value = self.eval_expr(value, locals)?;
                locals.insert(name.clone(), value);
                Ok(Control::Continue)
            }
            Stmt::Assign { target, value } => {
                let value = self.eval_expr(value, locals)?;
                let (root, path) = self.flatten_ast_place(target, locals)?;
                write_through_path(locals, &root, &path, value)?;
                Ok(Control::Continue)
            }
            Stmt::If {
                condition,
                then_body,
                else_body,
            } => {
                let condition = self.eval_expr(condition, locals)?;
                let Value::Bool(condition) = condition else {
                    return Err(runtime_error(
                        "RUNTIME_IF_CONDITION",
                        "if condition must evaluate to Bool",
                    ));
                };
                if condition {
                    self.eval_stmts(then_body, locals)
                } else {
                    self.eval_stmts(else_body, locals)
                }
            }
            Stmt::While { condition, body } => {
                loop {
                    self.loop_steps += 1;
                    if self.loop_steps > LOOP_LIMIT {
                        return Err(runtime_error(
                            "RUNTIME_LOOP_LIMIT",
                            "loop exceeded the Stage 0 execution step limit",
                        ));
                    }
                    let condition = self.eval_expr(condition, locals)?;
                    let Value::Bool(condition) = condition else {
                        return Err(runtime_error(
                            "RUNTIME_WHILE_CONDITION",
                            "while condition must evaluate to Bool",
                        ));
                    };
                    if !condition {
                        break;
                    }
                    match self.eval_stmts(body, locals)? {
                        Control::Continue => {}
                        Control::BreakLoop => break,
                        Control::ContinueLoop => continue,
                        returned @ Control::Return(_) => return Ok(returned),
                    }
                }
                Ok(Control::Continue)
            }
            Stmt::ForIn {
                binding,
                iterable,
                body,
                ..
            } => {
                if let Expr::Range { start, end, .. } = iterable {
                    let start_value = self.eval_expr(start, locals)?;
                    let end_value = self.eval_expr(end, locals)?;
                    let (Value::Int(start_value), Value::Int(end_value)) = (start_value, end_value)
                    else {
                        return Err(runtime_error(
                            "RUNTIME_FOR_RANGE_BOUND",
                            "for-in range bounds must evaluate to Int",
                        ));
                    };
                    let saved = locals.remove(binding);
                    let mut control = Control::Continue;
                    let mut i = start_value;
                    while i < end_value {
                        self.loop_steps += 1;
                        if self.loop_steps > LOOP_LIMIT {
                            if let Some(saved) = saved {
                                locals.insert(binding.clone(), saved);
                            } else {
                                locals.remove(binding);
                            }
                            return Err(runtime_error(
                                "RUNTIME_LOOP_LIMIT",
                                "loop exceeded the Stage 0 execution step limit",
                            ));
                        }
                        locals.insert(binding.clone(), Value::Int(i));
                        match self.eval_stmts(body, locals)? {
                            Control::Continue => {}
                            Control::BreakLoop => break,
                            Control::ContinueLoop => {
                                i += 1;
                                continue;
                            }
                            returned @ Control::Return(_) => {
                                control = returned;
                                break;
                            }
                        }
                        i += 1;
                    }
                    if let Some(saved) = saved {
                        locals.insert(binding.clone(), saved);
                    } else {
                        locals.remove(binding);
                    }
                    return Ok(control);
                }
                let iterable = self.eval_expr(iterable, locals)?;
                let Value::Array(elements) = iterable else {
                    return Err(runtime_error(
                        "RUNTIME_FOR_ITERABLE",
                        "for-in iterable must evaluate to an array",
                    ));
                };
                let saved = locals.remove(binding);
                let mut control = Control::Continue;
                for element in elements.iter() {
                    self.loop_steps += 1;
                    if self.loop_steps > LOOP_LIMIT {
                        if let Some(saved) = saved {
                            locals.insert(binding.clone(), saved);
                        } else {
                            locals.remove(binding);
                        }
                        return Err(runtime_error(
                            "RUNTIME_LOOP_LIMIT",
                            "loop exceeded the Stage 0 execution step limit",
                        ));
                    }
                    locals.insert(binding.clone(), element.clone());
                    match self.eval_stmts(body, locals)? {
                        Control::Continue => {}
                        Control::BreakLoop => break,
                        Control::ContinueLoop => continue,
                        returned @ Control::Return(_) => {
                            control = returned;
                            break;
                        }
                    }
                }
                if let Some(saved) = saved {
                    locals.insert(binding.clone(), saved);
                } else {
                    locals.remove(binding);
                }
                Ok(control)
            }
            Stmt::ForC {
                init,
                condition,
                step,
                body,
            } => {
                match self.eval_stmt(init, locals)? {
                    Control::Continue => {}
                    other => return Ok(other),
                }
                loop {
                    self.loop_steps += 1;
                    if self.loop_steps > LOOP_LIMIT {
                        return Err(runtime_error(
                            "RUNTIME_LOOP_LIMIT",
                            "loop exceeded the Stage 0 execution step limit",
                        ));
                    }
                    let condition = self.eval_expr(condition, locals)?;
                    let Value::Bool(condition) = condition else {
                        return Err(runtime_error(
                            "RUNTIME_FORC_CONDITION",
                            "for condition must evaluate to Bool",
                        ));
                    };
                    if !condition {
                        break;
                    }
                    match self.eval_stmts(body, locals)? {
                        Control::Continue => {}
                        Control::BreakLoop => break,
                        Control::ContinueLoop => {}
                        returned @ Control::Return(_) => return Ok(returned),
                    }
                    match self.eval_stmt(step, locals)? {
                        Control::Continue => {}
                        returned @ Control::Return(_) => return Ok(returned),
                        Control::BreakLoop | Control::ContinueLoop => {}
                    }
                }
                Ok(Control::Continue)
            }
            Stmt::Match { value, arms } => {
                let value = self.eval_expr(value, locals)?;
                for arm in arms {
                    if let Some(bindings) = pattern_bindings(&arm.pattern, &value)? {
                        let saved = apply_bindings(locals, bindings);
                        let result = self.eval_stmts(&arm.body, locals);
                        restore_bindings(locals, saved);
                        return result;
                    }
                }
                Err(runtime_error(
                    "RUNTIME_MATCH_NON_EXHAUSTIVE",
                    "match did not select an arm",
                ))
            }
            Stmt::Return(Some(value)) => Ok(Control::Return(self.eval_expr(value, locals)?)),
            Stmt::Return(None) => Ok(Control::Return(Value::Unit)),
            Stmt::Break { .. } => Ok(Control::BreakLoop),
            Stmt::Continue { .. } => Ok(Control::ContinueLoop),
            Stmt::Expr(value) => {
                let _ = self.eval_expr(value, locals)?;
                Ok(Control::Continue)
            }
        }
    }

    fn flatten_ast_place(
        &mut self,
        target: &Expr,
        locals: &HashMap<String, Value>,
    ) -> Result<(String, Vec<PlaceStep>), Diagnostic> {
        match target {
            Expr::Name { name, .. } => Ok((name.clone(), Vec::new())),
            Expr::FieldAccess { base, field, .. } => {
                let (root, mut path) = self.flatten_ast_place(base, locals)?;
                path.push(PlaceStep::Field(field.clone()));
                Ok((root, path))
            }
            Expr::Index { base, index, .. } => {
                let (root, mut path) = self.flatten_ast_place(base, locals)?;
                let idx = self.eval_expr(index, locals)?;
                let Value::Int(i) = idx else {
                    return Err(runtime_error(
                        "RUNTIME_ASSIGN_INDEX_TYPE",
                        "assignment index must evaluate to Int",
                    ));
                };
                if i < 0 {
                    return Err(runtime_error(
                        "RUNTIME_ASSIGN_INDEX_BOUNDS",
                        "negative assignment index",
                    ));
                }
                path.push(PlaceStep::Index(i as usize));
                Ok((root, path))
            }
            _ => Err(runtime_error(
                "RUNTIME_ASSIGN_TARGET",
                "invalid assignment target",
            )),
        }
    }

    /// Apply a closure (AST interpreter): the body is AST since an AST run only
    /// mints AST-bodied closures.
    fn apply_closure(&mut self, closure: &Closure, args: Vec<Value>) -> Result<Value, Diagnostic> {
        if closure.params.len() != args.len() {
            return Err(runtime_error(
                "RUNTIME_CALL_ARITY",
                format!(
                    "closure expects {} arguments, found {}",
                    closure.params.len(),
                    args.len()
                ),
            ));
        }
        let ClosureBody::Ast(body) = &closure.body else {
            return Err(runtime_error(
                "RUNTIME_CLOSURE_BODY",
                "closure body is not AST in the AST interpreter",
            ));
        };
        let mut call_locals = closure.captured.clone();
        for (name, value) in closure.params.iter().zip(args) {
            call_locals.insert(name.clone(), value);
        }
        self.eval_expr(body, &call_locals)
    }

    fn eval_expr(
        &mut self,
        expr: &Expr,
        locals: &HashMap<String, Value>,
    ) -> Result<Value, Diagnostic> {
        match expr {
            Expr::Try { .. } => unreachable!("`?` is desugared before evaluation"),
            Expr::Name { name, .. } => locals.get(name).cloned().ok_or_else(|| {
                runtime_error("RUNTIME_UNKNOWN_NAME", format!("unknown name `{name}`"))
            }),
            Expr::Int { value, .. } => value.parse::<i64>().map(Value::Int).map_err(|_| {
                runtime_error(
                    "RUNTIME_INT_PARSE",
                    format!("invalid Int literal `{value}`"),
                )
            }),
            Expr::Float { value, .. } => value.parse::<f64>().map(Value::Float).map_err(|_| {
                runtime_error(
                    "RUNTIME_FLOAT_PARSE",
                    format!("invalid Float literal `{value}`"),
                )
            }),
            Expr::String { value, .. } => Ok(Value::String(value.clone())),
            Expr::Bool { value, .. } => Ok(Value::Bool(*value)),
            Expr::Binary {
                op, left, right, ..
            } => self.eval_binary_expr(*op, left, right, locals),
            Expr::Unary { op, expr, .. } => {
                let value = self.eval_expr(expr, locals)?;
                eval_unary(*op, value)
            }
            Expr::Call { callee, args, .. } => {
                if is_std_builtin(callee) {
                    let args = args
                        .iter()
                        .map(|arg| self.eval_expr(arg, locals))
                        .collect::<Result<Vec<_>, _>>()?;
                    return eval_std_builtin(callee, args);
                }
                if let Some((enum_name, variant)) = callee.rsplit_once('.') {
                    if self
                        .enum_variants
                        .get(enum_name)
                        .is_some_and(|variants| variants.contains_key(variant))
                    {
                        return Ok(Value::Enum {
                            ty: enum_name.to_string(),
                            variant: variant.to_string(),
                            payload: args
                                .first()
                                .map(|arg| self.eval_expr(arg, locals).map(Box::new))
                                .transpose()?,
                        });
                    }
                }
                let Some(function) = self.functions.get(callee).cloned() else {
                    // Indirect call: `callee` may name a local closure value.
                    if let Some(Value::Closure(closure)) = locals.get(callee).cloned() {
                        let mut arg_values = Vec::with_capacity(args.len());
                        for arg in args {
                            arg_values.push(self.eval_expr(arg, locals)?);
                        }
                        return self.apply_closure(&closure, arg_values);
                    }
                    return Err(runtime_error(
                        "RUNTIME_UNKNOWN_FUNCTION",
                        format!("unknown function `{callee}`"),
                    ));
                };
                if function.params.len() != args.len() {
                    return Err(runtime_error(
                        "RUNTIME_CALL_ARITY",
                        format!(
                            "function `{callee}` expects {} arguments, found {}",
                            function.params.len(),
                            args.len()
                        ),
                    ));
                }
                let mut call_locals = HashMap::new();
                for (param, arg) in function.params.iter().zip(args) {
                    call_locals.insert(param.name.clone(), self.eval_expr(arg, locals)?);
                }
                match self.eval_stmts(&function.body, &mut call_locals)? {
                    Control::Return(value) => Ok(value),
                    Control::Continue => Ok(Value::Unit),
                    Control::BreakLoop => Err(runtime_error(
                        "RUNTIME_BREAK_OUTSIDE_LOOP",
                        "`break` reached function boundary",
                    )),
                    Control::ContinueLoop => Err(runtime_error(
                        "RUNTIME_CONTINUE_OUTSIDE_LOOP",
                        "`continue` reached function boundary",
                    )),
                }
            }
            Expr::StructLiteral { ty, fields, .. } => {
                let mut values = BTreeMap::new();
                for field in fields {
                    values.insert(field.name.clone(), self.eval_expr(&field.value, locals)?);
                }
                Ok(Value::Struct {
                    ty: ty.clone(),
                    fields: Rc::new(values),
                })
            }
            Expr::FieldAccess { base, field, .. } => {
                if let Expr::Name {
                    name: enum_name, ..
                } = base.as_ref()
                {
                    if let Some(variants) = self.enum_variants.get(enum_name) {
                        if variants.contains_key(field) {
                            return Ok(Value::Enum {
                                ty: enum_name.clone(),
                                variant: field.clone(),
                                payload: None,
                            });
                        }
                        return Err(runtime_error(
                            "RUNTIME_UNKNOWN_VARIANT",
                            format!("unknown variant `{field}` on enum `{enum_name}`"),
                        ));
                    }
                }
                let value = self.eval_expr(base, locals)?;
                if let Value::Array(values) = &value {
                    if field == "len" {
                        return Ok(Value::Int(values.len() as i64));
                    }
                    return Err(runtime_error(
                        "RUNTIME_ARRAY_FIELD",
                        format!("unknown field `{field}` on array; only `len` is supported"),
                    ));
                }
                if let Value::Tuple(values) = &value {
                    return tuple_field(values, field);
                }
                let Value::Struct { ty, fields } = value else {
                    return Err(runtime_error(
                        "RUNTIME_FIELD_BASE",
                        "field access requires a struct value",
                    ));
                };
                fields.get(field).cloned().ok_or_else(|| {
                    runtime_error(
                        "RUNTIME_UNKNOWN_FIELD",
                        format!("unknown field `{field}` on struct `{ty}`"),
                    )
                })
            }
            Expr::ArrayLiteral { elements, .. } => elements
                .iter()
                .map(|element| self.eval_expr(element, locals))
                .collect::<Result<Vec<_>, _>>()
                .map(|elements| Value::Array(Rc::new(elements))),
            Expr::Tuple { elements, .. } => elements
                .iter()
                .map(|element| self.eval_expr(element, locals))
                .collect::<Result<Vec<_>, _>>()
                .map(|elements| Value::Tuple(Rc::new(elements))),
            Expr::Lambda { params, body, .. } => {
                Ok(Value::Closure(Rc::new(Closure {
                    params: params.iter().map(|p| p.name.clone()).collect(),
                    captured: locals.clone(),
                    body: ClosureBody::Ast((**body).clone()),
                })))
            }
            Expr::Index { base, index, .. } => {
                let base = self.eval_expr(base, locals)?;
                let index = self.eval_expr(index, locals)?;
                index_array_value(base, index)
            }
            Expr::Range { .. } => Err(runtime_error(
                "RUNTIME_RANGE_EXPR",
                "range expression is only valid as a for-in iterable",
            )),
        }
    }

    fn eval_binary_expr(
        &mut self,
        op: BinaryOp,
        left: &Expr,
        right: &Expr,
        locals: &HashMap<String, Value>,
    ) -> Result<Value, Diagnostic> {
        match op {
            BinaryOp::And => {
                let left = expect_bool(self.eval_expr(left, locals)?, "RUNTIME_LOGICAL_OPERAND")?;
                if !left {
                    return Ok(Value::Bool(false));
                }
                Ok(Value::Bool(expect_bool(
                    self.eval_expr(right, locals)?,
                    "RUNTIME_LOGICAL_OPERAND",
                )?))
            }
            BinaryOp::Or => {
                let left = expect_bool(self.eval_expr(left, locals)?, "RUNTIME_LOGICAL_OPERAND")?;
                if left {
                    return Ok(Value::Bool(true));
                }
                Ok(Value::Bool(expect_bool(
                    self.eval_expr(right, locals)?,
                    "RUNTIME_LOGICAL_OPERAND",
                )?))
            }
            _ => {
                let left = self.eval_expr(left, locals)?;
                let right = self.eval_expr(right, locals)?;
                eval_binary(op, left, right)
            }
        }
    }
}

enum Control {
    Continue,
    BreakLoop,
    ContinueLoop,
    Return(Value),
}

fn eval_binary(op: BinaryOp, left: Value, right: Value) -> Result<Value, Diagnostic> {
    match op {
        BinaryOp::Eq => Ok(Value::Bool(left == right)),
        BinaryOp::NotEq => Ok(Value::Bool(left != right)),
        BinaryOp::And | BinaryOp::Or => {
            let left = expect_bool(left, "RUNTIME_LOGICAL_OPERAND")?;
            let right = expect_bool(right, "RUNTIME_LOGICAL_OPERAND")?;
            match op {
                BinaryOp::And => Ok(Value::Bool(left && right)),
                BinaryOp::Or => Ok(Value::Bool(left || right)),
                _ => unreachable!(),
            }
        }
        BinaryOp::Add
        | BinaryOp::Sub
        | BinaryOp::Mul
        | BinaryOp::Div
        | BinaryOp::Mod
        | BinaryOp::BitAnd
        | BinaryOp::BitOr
        | BinaryOp::BitXor => match (left, right) {
            (Value::Int(left), Value::Int(right)) => match op {
                BinaryOp::Add => Ok(Value::Int(left + right)),
                BinaryOp::Sub => Ok(Value::Int(left - right)),
                BinaryOp::Mul => Ok(Value::Int(left * right)),
                BinaryOp::BitAnd => Ok(Value::Int(left & right)),
                BinaryOp::BitOr => Ok(Value::Int(left | right)),
                BinaryOp::BitXor => Ok(Value::Int(left ^ right)),
                BinaryOp::Div => {
                    if right == 0 {
                        Err(runtime_error("RUNTIME_DIVIDE_BY_ZERO", "division by zero"))
                    } else {
                        Ok(Value::Int(left / right))
                    }
                }
                BinaryOp::Mod => {
                    if right == 0 {
                        Err(runtime_error("RUNTIME_DIVIDE_BY_ZERO", "modulo by zero"))
                    } else {
                        Ok(Value::Int(left % right))
                    }
                }
                _ => unreachable!(),
            },
            // Float arithmetic follows IEEE-754 (div by zero → inf/NaN, no
            // trap). Mod / bitwise are Int-only (rejected by typecheck).
            (Value::Float(left), Value::Float(right)) => match op {
                BinaryOp::Add => Ok(Value::Float(left + right)),
                BinaryOp::Sub => Ok(Value::Float(left - right)),
                BinaryOp::Mul => Ok(Value::Float(left * right)),
                BinaryOp::Div => Ok(Value::Float(left / right)),
                _ => Err(runtime_error(
                    "RUNTIME_BINARY_OPERAND",
                    "modulo / bitwise operators are not defined on Float",
                )),
            },
            _ => Err(runtime_error(
                "RUNTIME_BINARY_OPERAND",
                "binary arithmetic operands must both be Int or both be Float",
            )),
        },
        BinaryOp::Lt | BinaryOp::Lte | BinaryOp::Gt | BinaryOp::Gte => {
            let order = match (left, right) {
                (Value::Int(left), Value::Int(right)) => left.partial_cmp(&right),
                (Value::Float(left), Value::Float(right)) => left.partial_cmp(&right),
                _ => {
                    return Err(runtime_error(
                        "RUNTIME_BINARY_OPERAND",
                        "binary ordering operands must both be Int or both be Float",
                    ))
                }
            };
            let lt = order == Some(std::cmp::Ordering::Less);
            let gt = order == Some(std::cmp::Ordering::Greater);
            Ok(Value::Bool(match op {
                BinaryOp::Lt => lt,
                BinaryOp::Lte => !gt && order.is_some(),
                BinaryOp::Gt => gt,
                BinaryOp::Gte => !lt && order.is_some(),
                _ => unreachable!(),
            }))
        }
    }
}

/// 沿展平后的 place 路径定位到目标位置并原地写入。两套 interpreter 共用。
fn write_through_path(
    locals: &mut HashMap<String, Value>,
    root: &str,
    path: &[PlaceStep],
    value: Value,
) -> Result<(), Diagnostic> {
    if path.is_empty() {
        // Plain `name = value`: bind the slot directly. The RHS may have moved
        // `name` out (a last-use move, e.g. the common `name = f(name)`), so we
        // must not require the old slot to still exist — we're overwriting it.
        locals.insert(root.to_string(), value);
        return Ok(());
    }
    let mut slot = locals
        .get_mut(root)
        .ok_or_else(|| runtime_error("RUNTIME_UNKNOWN_NAME", format!("unknown name `{root}`")))?;
    for step in path {
        slot = match step {
            PlaceStep::Field(field) => match slot {
                Value::Struct { fields, .. } => {
                    Rc::make_mut(fields).get_mut(field).ok_or_else(|| {
                        runtime_error("RUNTIME_ASSIGN_FIELD", format!("unknown field `{field}`"))
                    })?
                }
                _ => {
                    return Err(runtime_error(
                        "RUNTIME_ASSIGN_FIELD_BASE",
                        "field assignment requires a struct value",
                    ))
                }
            },
            PlaceStep::Index(i) => match slot {
                Value::Array(values) => {
                    let values = Rc::make_mut(values);
                    if *i >= values.len() {
                        return Err(runtime_error(
                            "RUNTIME_ASSIGN_INDEX_BOUNDS",
                            "assignment index out of bounds",
                        ));
                    }
                    &mut values[*i]
                }
                _ => {
                    return Err(runtime_error(
                        "RUNTIME_ASSIGN_INDEX_BASE",
                        "index assignment requires an array value",
                    ))
                }
            },
        };
    }
    *slot = value;
    Ok(())
}

fn eval_unary(op: UnaryOp, value: Value) -> Result<Value, Diagnostic> {
    match op {
        UnaryOp::Not => Ok(Value::Bool(!expect_bool(value, "RUNTIME_UNARY_OPERAND")?)),
        UnaryOp::Neg => match value {
            Value::Float(f) => Ok(Value::Float(-f)),
            other => Ok(Value::Int(-expect_int(other, "RUNTIME_UNARY_OPERAND")?)),
        },
        UnaryOp::BitNot => Ok(Value::Int(!expect_int(value, "RUNTIME_UNARY_OPERAND")?)),
    }
}

fn expect_int(value: Value, code: &'static str) -> Result<i64, Diagnostic> {
    let Value::Int(value) = value else {
        return Err(runtime_error(code, "operand must evaluate to Int"));
    };
    Ok(value)
}

fn expect_bool(value: Value, code: &'static str) -> Result<bool, Diagnostic> {
    let Value::Bool(value) = value else {
        return Err(runtime_error(code, "operand must evaluate to Bool"));
    };
    Ok(value)
}

fn tuple_field(values: &[Value], field: &str) -> Result<Value, Diagnostic> {
    match field.parse::<usize>() {
        Ok(index) if index < values.len() => Ok(values[index].clone()),
        _ => Err(runtime_error(
            "RUNTIME_TUPLE_INDEX",
            format!(
                "tuple index `.{field}` out of range for {}-element tuple",
                values.len()
            ),
        )),
    }
}

fn index_array_value(base: Value, index: Value) -> Result<Value, Diagnostic> {
    let Value::Array(values) = base else {
        return Err(runtime_error(
            "RUNTIME_INDEX_BASE",
            "index expression requires an array value",
        ));
    };
    let Value::Int(index) = index else {
        return Err(runtime_error("RUNTIME_INDEX", "array index must be Int"));
    };
    if index < 0 {
        return Err(runtime_error(
            "RUNTIME_INDEX_BOUNDS",
            format!("array index `{index}` is out of bounds"),
        ));
    }
    values.get(index as usize).cloned().ok_or_else(|| {
        runtime_error(
            "RUNTIME_INDEX_BOUNDS",
            format!(
                "array index `{index}` is out of bounds for length {}",
                values.len()
            ),
        )
    })
}

fn is_std_builtin(callee: &str) -> bool {
    matches!(
        callee,
        "string_len"
            | "string_byte_at"
            | "string_byte_slice"
            | "string_concat"
            | "int_to_string"
            | "int_abs"
            | "int_min"
            | "int_max"
            | "string_index_of"
            | "string_contains"
            | "string_repeat"
            | "ascii_is_digit"
            | "ascii_is_alpha"
            | "ascii_is_alnum"
            | "ascii_is_whitespace"
            | "int_array_empty"
            | "int_array_push"
            | "string_array_empty"
            | "string_array_push"
            | "bool_array_empty"
            | "bool_array_push"
            | "float_array_empty"
            | "float_array_push"
            | "file_read_to_string"
            | "path_join"
            | "path_basename"
            | "diagnostic_format"
    )
}

/// Byte index of the first occurrence of `needle` in `haystack`, or -1. An empty
/// needle matches at 0. Native codegen mirrors this exact naive scan so the
/// differential oracle holds.
fn byte_index_of(haystack: &[u8], needle: &[u8]) -> i64 {
    if needle.is_empty() {
        return 0;
    }
    if needle.len() > haystack.len() {
        return -1;
    }
    let last = haystack.len() - needle.len();
    let mut i = 0;
    while i <= last {
        if &haystack[i..i + needle.len()] == needle {
            return i as i64;
        }
        i += 1;
    }
    -1
}

fn eval_std_builtin(callee: &str, args: Vec<Value>) -> Result<Value, Diagnostic> {
    match callee {
        "string_len" => {
            let [value]: [Value; 1] = expect_arity(callee, args)?.try_into().ok().unwrap();
            let Value::String(value) = value else {
                return Err(runtime_error(
                    "RUNTIME_STD_TYPE",
                    "string_len expects String",
                ));
            };
            Ok(Value::Int(value.len() as i64))
        }
        "string_byte_at" => {
            let [value, index]: [Value; 2] = expect_arity(callee, args)?.try_into().ok().unwrap();
            let Value::String(value) = value else {
                return Err(runtime_error(
                    "RUNTIME_STD_TYPE",
                    "string_byte_at expects String",
                ));
            };
            let Value::Int(index) = index else {
                return Err(runtime_error(
                    "RUNTIME_STD_TYPE",
                    "string_byte_at index expects Int",
                ));
            };
            if index < 0 {
                return Err(runtime_error(
                    "RUNTIME_STRING_INDEX",
                    format!("string byte index `{index}` is out of bounds"),
                ));
            }
            value
                .as_bytes()
                .get(index as usize)
                .map(|byte| Value::Int(i64::from(*byte)))
                .ok_or_else(|| {
                    runtime_error(
                        "RUNTIME_STRING_INDEX",
                        format!(
                            "string byte index `{index}` is out of bounds for length {}",
                            value.len()
                        ),
                    )
                })
        }
        "string_byte_slice" => {
            let [value, start, len]: [Value; 3] =
                expect_arity(callee, args)?.try_into().ok().unwrap();
            let Value::String(value) = value else {
                return Err(runtime_error(
                    "RUNTIME_STD_TYPE",
                    "string_byte_slice expects String",
                ));
            };
            let Value::Int(start) = start else {
                return Err(runtime_error(
                    "RUNTIME_STD_TYPE",
                    "string_byte_slice start expects Int",
                ));
            };
            let Value::Int(len) = len else {
                return Err(runtime_error(
                    "RUNTIME_STD_TYPE",
                    "string_byte_slice len expects Int",
                ));
            };
            if start < 0 || len < 0 {
                return Err(runtime_error(
                    "RUNTIME_STRING_SLICE",
                    "string_byte_slice start and len must be non-negative",
                ));
            }
            let start = start as usize;
            let end = start.saturating_add(len as usize);
            value
                .get(start..end)
                .map(|slice| Value::String(slice.to_string()))
                .ok_or_else(|| {
                    runtime_error(
                        "RUNTIME_STRING_SLICE",
                        format!(
                            "string byte slice `{start}..{end}` is out of bounds or splits utf-8"
                        ),
                    )
                })
        }
        "string_concat" => {
            let [left, right]: [Value; 2] = expect_arity(callee, args)?.try_into().ok().unwrap();
            let Value::String(left) = left else {
                return Err(runtime_error(
                    "RUNTIME_STD_TYPE",
                    "string_concat left expects String",
                ));
            };
            let Value::String(right) = right else {
                return Err(runtime_error(
                    "RUNTIME_STD_TYPE",
                    "string_concat right expects String",
                ));
            };
            Ok(Value::String(format!("{left}{right}")))
        }
        "int_to_string" => {
            let [value]: [Value; 1] = expect_arity(callee, args)?.try_into().ok().unwrap();
            let Value::Int(value) = value else {
                return Err(runtime_error(
                    "RUNTIME_STD_TYPE",
                    "int_to_string expects Int",
                ));
            };
            Ok(Value::String(value.to_string()))
        }
        "int_abs" => {
            let [v]: [Value; 1] = expect_arity(callee, args)?.try_into().ok().unwrap();
            let Value::Int(v) = v else {
                return Err(runtime_error("RUNTIME_STD_TYPE", "int_abs expects Int"));
            };
            Ok(Value::Int(v.wrapping_abs()))
        }
        "int_min" => {
            let [a, b]: [Value; 2] = expect_arity(callee, args)?.try_into().ok().unwrap();
            let (Value::Int(a), Value::Int(b)) = (a, b) else {
                return Err(runtime_error("RUNTIME_STD_TYPE", "int_min expects Int, Int"));
            };
            Ok(Value::Int(a.min(b)))
        }
        "int_max" => {
            let [a, b]: [Value; 2] = expect_arity(callee, args)?.try_into().ok().unwrap();
            let (Value::Int(a), Value::Int(b)) = (a, b) else {
                return Err(runtime_error("RUNTIME_STD_TYPE", "int_max expects Int, Int"));
            };
            Ok(Value::Int(a.max(b)))
        }
        "string_index_of" => {
            let [s, sub]: [Value; 2] = expect_arity(callee, args)?.try_into().ok().unwrap();
            let (Value::String(s), Value::String(sub)) = (s, sub) else {
                return Err(runtime_error(
                    "RUNTIME_STD_TYPE",
                    "string_index_of expects String, String",
                ));
            };
            Ok(Value::Int(byte_index_of(s.as_bytes(), sub.as_bytes())))
        }
        "string_contains" => {
            let [s, sub]: [Value; 2] = expect_arity(callee, args)?.try_into().ok().unwrap();
            let (Value::String(s), Value::String(sub)) = (s, sub) else {
                return Err(runtime_error(
                    "RUNTIME_STD_TYPE",
                    "string_contains expects String, String",
                ));
            };
            Ok(Value::Bool(byte_index_of(s.as_bytes(), sub.as_bytes()) >= 0))
        }
        "string_repeat" => {
            let [s, n]: [Value; 2] = expect_arity(callee, args)?.try_into().ok().unwrap();
            let (Value::String(s), Value::Int(n)) = (s, n) else {
                return Err(runtime_error(
                    "RUNTIME_STD_TYPE",
                    "string_repeat expects String, Int",
                ));
            };
            let count = if n < 0 { 0 } else { n as usize };
            Ok(Value::String(s.repeat(count)))
        }
        "ascii_is_digit" => eval_ascii_predicate(callee, args, |byte| byte.is_ascii_digit()),
        "ascii_is_alpha" => eval_ascii_predicate(callee, args, |byte| byte.is_ascii_alphabetic()),
        "ascii_is_alnum" => eval_ascii_predicate(callee, args, |byte| byte.is_ascii_alphanumeric()),
        "ascii_is_whitespace" => {
            eval_ascii_predicate(callee, args, |byte| byte.is_ascii_whitespace())
        }
        "int_array_empty" | "string_array_empty" | "bool_array_empty" | "float_array_empty" => {
            let []: [Value; 0] = expect_arity(callee, args)?.try_into().ok().unwrap();
            Ok(Value::Array(Rc::new(Vec::new())))
        }
        "int_array_push" => eval_array_push(callee, args, "Int"),
        "string_array_push" => eval_array_push(callee, args, "String"),
        "bool_array_push" => eval_array_push(callee, args, "Bool"),
        "float_array_push" => eval_array_push(callee, args, "Float"),
        "file_read_to_string" => {
            let [path]: [Value; 1] = expect_arity(callee, args)?.try_into().ok().unwrap();
            let Value::String(path) = path else {
                return Err(runtime_error(
                    "RUNTIME_STD_TYPE",
                    "file_read_to_string expects String",
                ));
            };
            Ok(result_string_value(read_file_to_string(&path)))
        }
        "path_join" => {
            let [left, right]: [Value; 2] = expect_arity(callee, args)?.try_into().ok().unwrap();
            let Value::String(left) = left else {
                return Err(runtime_error(
                    "RUNTIME_STD_TYPE",
                    "path_join left expects String",
                ));
            };
            let Value::String(right) = right else {
                return Err(runtime_error(
                    "RUNTIME_STD_TYPE",
                    "path_join right expects String",
                ));
            };
            Ok(Value::String(join_path(&left, &right)))
        }
        "path_basename" => {
            let [path]: [Value; 1] = expect_arity(callee, args)?.try_into().ok().unwrap();
            let Value::String(path) = path else {
                return Err(runtime_error(
                    "RUNTIME_STD_TYPE",
                    "path_basename expects String",
                ));
            };
            Ok(Value::String(path_basename(&path)))
        }
        "diagnostic_format" => {
            let [code, line, column, message]: [Value; 4] =
                expect_arity(callee, args)?.try_into().ok().unwrap();
            let Value::String(code) = code else {
                return Err(runtime_error(
                    "RUNTIME_STD_TYPE",
                    "diagnostic_format code expects String",
                ));
            };
            let Value::Int(line) = line else {
                return Err(runtime_error(
                    "RUNTIME_STD_TYPE",
                    "diagnostic_format line expects Int",
                ));
            };
            let Value::Int(column) = column else {
                return Err(runtime_error(
                    "RUNTIME_STD_TYPE",
                    "diagnostic_format column expects Int",
                ));
            };
            let Value::String(message) = message else {
                return Err(runtime_error(
                    "RUNTIME_STD_TYPE",
                    "diagnostic_format message expects String",
                ));
            };
            Ok(Value::String(format!(
                "{code} at {line}:{column}: {message}"
            )))
        }
        _ => Err(runtime_error(
            "RUNTIME_UNKNOWN_FUNCTION",
            format!("unknown function `{callee}`"),
        )),
    }
}

fn result_string_value(result: Result<String, String>) -> Value {
    let (variant, payload) = match result {
        Ok(value) => ("Ok", value),
        Err(message) => ("Err", message),
    };
    Value::Enum {
        ty: "ResultString".to_string(),
        variant: variant.to_string(),
        payload: Some(Box::new(Value::String(payload))),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn read_file_to_string(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|err| err.to_string())
}

#[cfg(target_arch = "wasm32")]
fn read_file_to_string(_path: &str) -> Result<String, String> {
    Err("file io unavailable on wasm32".to_string())
}

fn join_path(left: &str, right: &str) -> String {
    if left.is_empty() {
        return right.to_string();
    }
    if right.is_empty() {
        return left.to_string();
    }
    if right.starts_with('/') || right.starts_with('\\') {
        return right.to_string();
    }
    if left.ends_with('/') || left.ends_with('\\') {
        format!("{left}{right}")
    } else {
        format!("{left}/{right}")
    }
}

fn path_basename(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .to_string()
}

fn eval_array_push(
    callee: &str,
    args: Vec<Value>,
    element_type: &'static str,
) -> Result<Value, Diagnostic> {
    let [array, value]: [Value; 2] = expect_arity(callee, args)?.try_into().ok().unwrap();
    let Value::Array(mut values) = array else {
        return Err(runtime_error(
            "RUNTIME_STD_TYPE",
            format!("{callee} expects array as first argument"),
        ));
    };
    // `make_mut` mutates in place when this is the sole owner (the common
    // `a.field = int_array_push(a.field, x)` arena idiom), copying only when
    // the backing array is still shared elsewhere.
    let target = Rc::make_mut(&mut values);
    match (element_type, &value) {
        ("Int", Value::Int(_))
        | ("String", Value::String(_))
        | ("Bool", Value::Bool(_))
        | ("Float", Value::Float(_)) => {}
        _ => {
            return Err(runtime_error(
                "RUNTIME_STD_TYPE",
                format!("{callee} expects {element_type} value"),
            ));
        }
    }
    target.push(value);
    Ok(Value::Array(values))
}

fn eval_ascii_predicate(
    callee: &str,
    args: Vec<Value>,
    predicate: impl Fn(u8) -> bool,
) -> Result<Value, Diagnostic> {
    let [value]: [Value; 1] = expect_arity(callee, args)?.try_into().ok().unwrap();
    let Value::Int(value) = value else {
        return Err(runtime_error(
            "RUNTIME_STD_TYPE",
            format!("{callee} expects Int"),
        ));
    };
    if !(0..=255).contains(&value) {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(predicate(value as u8)))
}

fn expect_arity(callee: &str, args: Vec<Value>) -> Result<Vec<Value>, Diagnostic> {
    let expected = match callee {
        "string_len" => 1,
        "string_byte_at" => 2,
        "string_byte_slice" => 3,
        "ascii_is_digit" | "ascii_is_alpha" | "ascii_is_alnum" | "ascii_is_whitespace" => 1,
        "int_array_empty" | "string_array_empty" | "bool_array_empty" | "float_array_empty" => 0,
        "int_array_push" | "string_array_push" | "bool_array_push" | "float_array_push" => 2,
        _ => args.len(),
    };
    if args.len() != expected {
        return Err(runtime_error(
            "RUNTIME_CALL_ARITY",
            format!(
                "function `{callee}` expects {expected} arguments, found {}",
                args.len()
            ),
        ));
    }
    Ok(args)
}

type BindingSnapshot = Vec<(String, Option<Value>)>;

fn apply_bindings(
    locals: &mut HashMap<String, Value>,
    bindings: HashMap<String, Value>,
) -> BindingSnapshot {
    bindings
        .into_iter()
        .map(|(name, value)| {
            let old = locals.insert(name.clone(), value);
            (name, old)
        })
        .collect()
}

fn restore_bindings(locals: &mut HashMap<String, Value>, saved: BindingSnapshot) {
    for (name, old) in saved {
        if let Some(old) = old {
            locals.insert(name, old);
        } else {
            locals.remove(&name);
        }
    }
}

fn mir_pattern_bindings(
    pattern: &MirPattern,
    value: &Value,
) -> Result<Option<HashMap<String, Value>>, Diagnostic> {
    match pattern {
        MirPattern::Name(name) => Ok(Some(HashMap::from([(name.clone(), value.clone())]))),
        MirPattern::Variant {
            enum_name,
            variant,
            binding,
        } => match value {
            Value::Enum {
                ty,
                variant: value_variant,
                payload,
            } if ty == enum_name && value_variant == variant => {
                let mut bindings = HashMap::new();
                if let Some(binding) = binding {
                    let Some(payload) = payload else {
                        return Err(runtime_error(
                            "RUNTIME_PATTERN_PAYLOAD",
                            format!("variant `{enum_name}.{variant}` has no payload to bind"),
                        ));
                    };
                    bindings.insert(binding.clone(), payload.as_ref().clone());
                }
                Ok(Some(bindings))
            }
            _ => Ok(None),
        },
        MirPattern::Int(pattern) => {
            let parsed = pattern.parse::<i64>().map_err(|_| {
                runtime_error(
                    "RUNTIME_INVALID_PATTERN",
                    format!("invalid Int match pattern `{pattern}`"),
                )
            })?;
            Ok(matches!(value, Value::Int(value) if *value == parsed).then(HashMap::new))
        }
        MirPattern::String(pattern) => {
            Ok(matches!(value, Value::String(value) if value == pattern).then(HashMap::new))
        }
        MirPattern::Bool(pattern) => {
            Ok(matches!(value, Value::Bool(value) if value == pattern).then(HashMap::new))
        }
        MirPattern::Wildcard => Ok(Some(HashMap::new())),
    }
}

fn pattern_bindings(
    pattern: &Pattern,
    value: &Value,
) -> Result<Option<HashMap<String, Value>>, Diagnostic> {
    match pattern {
        Pattern::Name(name) => Ok(Some(HashMap::from([(name.clone(), value.clone())]))),
        Pattern::Variant {
            enum_name,
            variant,
            binding,
        } => match value {
            Value::Enum {
                ty,
                variant: value_variant,
                payload,
            } if ty == enum_name && value_variant == variant => {
                let mut bindings = HashMap::new();
                if let Some(binding) = binding {
                    let Some(payload) = payload else {
                        return Err(runtime_error(
                            "RUNTIME_PATTERN_PAYLOAD",
                            format!("variant `{enum_name}.{variant}` has no payload to bind"),
                        ));
                    };
                    bindings.insert(binding.clone(), payload.as_ref().clone());
                }
                Ok(Some(bindings))
            }
            _ => Ok(None),
        },
        Pattern::Int(pattern) => {
            let parsed = pattern.parse::<i64>().map_err(|_| {
                runtime_error(
                    "RUNTIME_INVALID_PATTERN",
                    format!("invalid Int match pattern `{pattern}`"),
                )
            })?;
            Ok(matches!(value, Value::Int(value) if *value == parsed).then(HashMap::new))
        }
        Pattern::String(pattern) => {
            Ok(matches!(value, Value::String(value) if value == pattern).then(HashMap::new))
        }
        Pattern::Bool(pattern) => {
            Ok(matches!(value, Value::Bool(value) if value == pattern).then(HashMap::new))
        }
        Pattern::Wildcard => Ok(Some(HashMap::new())),
    }
}

fn runtime_error(code: &'static str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message, Span::new(0, 0))
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(value) => write!(f, "{value}"),
            // Always show a decimal point so a Float is unambiguous (1.0, not 1).
            Value::Float(value) => {
                let s = format!("{value}");
                if s.contains('.') || s.contains('e') || s.contains("inf") || s.contains("NaN") {
                    write!(f, "{s}")
                } else {
                    write!(f, "{s}.0")
                }
            }
            Value::String(value) => write!(f, "{value}"),
            Value::Bool(value) => write!(f, "{value}"),
            Value::Struct { ty, fields } => {
                let fields = fields
                    .iter()
                    .map(|(name, value)| format!("{name}: {value}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "{ty} {{ {fields} }}")
            }
            Value::Enum {
                ty,
                variant,
                payload,
            } => {
                if let Some(payload) = payload {
                    write!(f, "{ty}.{variant}({payload})")
                } else {
                    write!(f, "{ty}.{variant}")
                }
            }
            Value::Array(values) => {
                let values = values
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "[{values}]")
            }
            Value::Tuple(values) => {
                let values = values
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(f, "({values})")
            }
            Value::Closure(closure) => {
                write!(f, "<closure |{}|>", closure.params.join(", "))
            }
            Value::Unit => write!(f, "()"),
        }
    }
}
