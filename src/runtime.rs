use crate::ast::{BinaryOp, Expr, Function, Item, Module, Pattern, Stmt, UnaryOp};
use crate::diagnostic::{Diagnostic, Span};
use crate::mir::{self, MirExpr, MirFunction, MirPattern, MirPlace, MirStmt, Program};
use std::collections::{BTreeMap, HashMap};
use std::fmt;

const LOOP_LIMIT: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Int(i64),
    String(String),
    Bool(bool),
    Struct {
        ty: String,
        fields: BTreeMap<String, Value>,
    },
    Enum {
        ty: String,
        variant: String,
        payload: Option<Box<Value>>,
    },
    Array(Vec<Value>),
    Unit,
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
    runtime.call_function(main).map_err(|err| vec![err])
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

struct MirRuntime {
    functions: HashMap<String, MirFunction>,
    enum_variants: HashMap<String, HashMap<String, Option<String>>>,
    loop_steps: usize,
}

impl MirRuntime {
    fn new(program: &Program) -> Self {
        Self {
            functions: program
                .functions
                .iter()
                .map(|function| (function.name.clone(), function.clone()))
                .collect(),
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
        locals: &HashMap<String, Value>,
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

    fn eval_expr(
        &mut self,
        expr: &MirExpr,
        locals: &HashMap<String, Value>,
    ) -> Result<Value, Diagnostic> {
        match expr {
            MirExpr::Load(name) => locals.get(name).cloned().ok_or_else(|| {
                runtime_error("RUNTIME_UNKNOWN_NAME", format!("unknown name `{name}`"))
            }),
            MirExpr::Int(value) => value.parse::<i64>().map(Value::Int).map_err(|_| {
                runtime_error(
                    "RUNTIME_INT_PARSE",
                    format!("invalid Int literal `{value}`"),
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
                    let args = args
                        .iter()
                        .map(|arg| self.eval_expr(arg, locals))
                        .collect::<Result<Vec<_>, _>>()?;
                    return eval_std_builtin(callee, args);
                }
                let Some(function) = self.functions.get(callee).cloned() else {
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
            } => Ok(Value::Enum {
                ty: enum_name.clone(),
                variant: variant.clone(),
                payload: payload
                    .as_ref()
                    .map(|payload| self.eval_expr(payload, locals).map(Box::new))
                    .transpose()?,
            }),
            MirExpr::StructLiteral { ty, fields } => {
                let mut values = BTreeMap::new();
                for field in fields {
                    values.insert(field.name.clone(), self.eval_expr(&field.value, locals)?);
                }
                Ok(Value::Struct {
                    ty: ty.clone(),
                    fields: values,
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
            MirExpr::ArrayLiteral { elements } => elements
                .iter()
                .map(|element| self.eval_expr(element, locals))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
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

    fn eval_expr(
        &mut self,
        expr: &Expr,
        locals: &HashMap<String, Value>,
    ) -> Result<Value, Diagnostic> {
        match expr {
            Expr::Name { name, .. } => locals.get(name).cloned().ok_or_else(|| {
                runtime_error("RUNTIME_UNKNOWN_NAME", format!("unknown name `{name}`"))
            }),
            Expr::Int { value, .. } => value.parse::<i64>().map(Value::Int).map_err(|_| {
                runtime_error(
                    "RUNTIME_INT_PARSE",
                    format!("invalid Int literal `{value}`"),
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
                    fields: values,
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
                .map(Value::Array),
            Expr::Index { base, index, .. } => {
                let base = self.eval_expr(base, locals)?;
                let index = self.eval_expr(index, locals)?;
                index_array_value(base, index)
            }
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
        BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
            let (Value::Int(left), Value::Int(right)) = (left, right) else {
                return Err(runtime_error(
                    "RUNTIME_BINARY_OPERAND",
                    "binary arithmetic operands must evaluate to Int",
                ));
            };
            match op {
                BinaryOp::Add => Ok(Value::Int(left + right)),
                BinaryOp::Sub => Ok(Value::Int(left - right)),
                BinaryOp::Mul => Ok(Value::Int(left * right)),
                BinaryOp::Div => {
                    if right == 0 {
                        Err(runtime_error("RUNTIME_DIVIDE_BY_ZERO", "division by zero"))
                    } else {
                        Ok(Value::Int(left / right))
                    }
                }
                _ => unreachable!(),
            }
        }
        BinaryOp::Lt | BinaryOp::Lte | BinaryOp::Gt | BinaryOp::Gte => {
            let (Value::Int(left), Value::Int(right)) = (left, right) else {
                return Err(runtime_error(
                    "RUNTIME_BINARY_OPERAND",
                    "binary ordering operands must evaluate to Int",
                ));
            };
            match op {
                BinaryOp::Lt => Ok(Value::Bool(left < right)),
                BinaryOp::Lte => Ok(Value::Bool(left <= right)),
                BinaryOp::Gt => Ok(Value::Bool(left > right)),
                BinaryOp::Gte => Ok(Value::Bool(left >= right)),
                _ => unreachable!(),
            }
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
    let mut slot = locals
        .get_mut(root)
        .ok_or_else(|| runtime_error("RUNTIME_UNKNOWN_NAME", format!("unknown name `{root}`")))?;
    for step in path {
        slot = match step {
            PlaceStep::Field(field) => match slot {
                Value::Struct { fields, .. } => fields.get_mut(field).ok_or_else(|| {
                    runtime_error("RUNTIME_ASSIGN_FIELD", format!("unknown field `{field}`"))
                })?,
                _ => {
                    return Err(runtime_error(
                        "RUNTIME_ASSIGN_FIELD_BASE",
                        "field assignment requires a struct value",
                    ))
                }
            },
            PlaceStep::Index(i) => match slot {
                Value::Array(values) => {
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
        UnaryOp::Neg => Ok(Value::Int(-expect_int(value, "RUNTIME_UNARY_OPERAND")?)),
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
            | "file_read_to_string"
            | "path_join"
            | "path_basename"
            | "diagnostic_format"
    )
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
        "ascii_is_digit" => eval_ascii_predicate(callee, args, |byte| byte.is_ascii_digit()),
        "ascii_is_alpha" => eval_ascii_predicate(callee, args, |byte| byte.is_ascii_alphabetic()),
        "ascii_is_alnum" => eval_ascii_predicate(callee, args, |byte| byte.is_ascii_alphanumeric()),
        "ascii_is_whitespace" => {
            eval_ascii_predicate(callee, args, |byte| byte.is_ascii_whitespace())
        }
        "int_array_empty" | "string_array_empty" | "bool_array_empty" => {
            let []: [Value; 0] = expect_arity(callee, args)?.try_into().ok().unwrap();
            Ok(Value::Array(Vec::new()))
        }
        "int_array_push" => eval_array_push(callee, args, "Int"),
        "string_array_push" => eval_array_push(callee, args, "String"),
        "bool_array_push" => eval_array_push(callee, args, "Bool"),
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
    match (element_type, &value) {
        ("Int", Value::Int(_)) | ("String", Value::String(_)) | ("Bool", Value::Bool(_)) => {}
        _ => {
            return Err(runtime_error(
                "RUNTIME_STD_TYPE",
                format!("{callee} expects {element_type} value"),
            ));
        }
    }
    values.push(value);
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
        "int_array_empty" | "string_array_empty" | "bool_array_empty" => 0,
        "int_array_push" | "string_array_push" | "bool_array_push" => 2,
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
            Value::Unit => write!(f, "()"),
        }
    }
}
