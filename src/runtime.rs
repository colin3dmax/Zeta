use crate::ast::{BinaryOp, Expr, Function, Item, Module, Pattern, Stmt, UnaryOp};
use crate::diagnostic::{Diagnostic, Span};
use crate::mir::{self, MirExpr, MirFunction, MirPattern, MirStmt, Program};
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
    Unit,
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

    let mut runtime = MirRuntime::new(program);
    runtime.call_function(main).map_err(|err| vec![err])
}

#[derive(Debug, Default)]
pub struct ReplSession {
    locals: HashMap<String, Value>,
    functions: HashMap<String, Function>,
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

        let mut runtime = Runtime::from_functions(self.functions.clone());
        match runtime.eval_stmts(&main.body, &mut self.locals) {
            Ok(Control::Return(value)) => Ok(value),
            Ok(Control::Continue) => Ok(Value::Unit),
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
            loop_steps: 0,
        }
    }

    fn call_function(&mut self, function: &MirFunction) -> Result<Value, Diagnostic> {
        let mut locals = HashMap::new();
        match self.eval_stmts(&function.body, &mut locals)? {
            Control::Return(value) => Ok(value),
            Control::Continue => Ok(Value::Unit),
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
                returned @ Control::Return(_) => return Ok(returned),
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
            MirStmt::Store { name, value } => {
                let value = self.eval_expr(value, locals)?;
                if !locals.contains_key(name) {
                    return Err(runtime_error(
                        "RUNTIME_UNKNOWN_NAME",
                        format!("unknown name `{name}`"),
                    ));
                }
                locals.insert(name.clone(), value);
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
                        returned @ Control::Return(_) => return Ok(returned),
                    }
                }
                Ok(Control::Continue)
            }
            MirStmt::Match { value, arms } => {
                let value = self.eval_expr(value, locals)?;
                for arm in arms {
                    if mir_pattern_matches(&arm.pattern, &value)? {
                        return self.eval_stmts(&arm.body, locals);
                    }
                }
                Err(runtime_error(
                    "RUNTIME_MATCH_NON_EXHAUSTIVE",
                    "match did not select an arm",
                ))
            }
            MirStmt::Return(Some(value)) => Ok(Control::Return(self.eval_expr(value, locals)?)),
            MirStmt::Return(None) => Ok(Control::Return(Value::Unit)),
            MirStmt::Drop(value) => {
                let _ = self.eval_expr(value, locals)?;
                Ok(Control::Continue)
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
                }
            }
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
                let value = self.eval_expr(base, locals)?;
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
    loop_steps: usize,
}

impl Runtime {
    fn from_functions(functions: HashMap<String, Function>) -> Self {
        Self {
            functions,
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
                returned @ Control::Return(_) => return Ok(returned),
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
            Stmt::Assign { name, value, .. } => {
                let value = self.eval_expr(value, locals)?;
                if !locals.contains_key(name) {
                    return Err(runtime_error(
                        "RUNTIME_UNKNOWN_NAME",
                        format!("unknown name `{name}`"),
                    ));
                }
                locals.insert(name.clone(), value);
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
                        returned @ Control::Return(_) => return Ok(returned),
                    }
                }
                Ok(Control::Continue)
            }
            Stmt::Match { value, arms } => {
                let value = self.eval_expr(value, locals)?;
                for arm in arms {
                    if pattern_matches(&arm.pattern, &value)? {
                        return self.eval_stmts(&arm.body, locals);
                    }
                }
                Err(runtime_error(
                    "RUNTIME_MATCH_NON_EXHAUSTIVE",
                    "match did not select an arm",
                ))
            }
            Stmt::Return(Some(value)) => Ok(Control::Return(self.eval_expr(value, locals)?)),
            Stmt::Return(None) => Ok(Control::Return(Value::Unit)),
            Stmt::Expr(value) => {
                let _ = self.eval_expr(value, locals)?;
                Ok(Control::Continue)
            }
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
                let value = self.eval_expr(base, locals)?;
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

fn eval_unary(op: UnaryOp, value: Value) -> Result<Value, Diagnostic> {
    match op {
        UnaryOp::Not => Ok(Value::Bool(!expect_bool(value, "RUNTIME_UNARY_OPERAND")?)),
    }
}

fn expect_bool(value: Value, code: &'static str) -> Result<bool, Diagnostic> {
    let Value::Bool(value) = value else {
        return Err(runtime_error(code, "operand must evaluate to Bool"));
    };
    Ok(value)
}

fn mir_pattern_matches(pattern: &MirPattern, value: &Value) -> Result<bool, Diagnostic> {
    match pattern {
        MirPattern::Name(name) => Err(runtime_error(
            "RUNTIME_UNSUPPORTED_PATTERN",
            format!("name pattern `{name}` is not executable yet"),
        )),
        MirPattern::Int(pattern) => {
            let parsed = pattern.parse::<i64>().map_err(|_| {
                runtime_error(
                    "RUNTIME_INVALID_PATTERN",
                    format!("invalid Int match pattern `{pattern}`"),
                )
            })?;
            Ok(matches!(value, Value::Int(value) if *value == parsed))
        }
        MirPattern::String(pattern) => {
            Ok(matches!(value, Value::String(value) if value == pattern))
        }
        MirPattern::Bool(pattern) => Ok(matches!(value, Value::Bool(value) if value == pattern)),
        MirPattern::Wildcard => Ok(true),
    }
}

fn pattern_matches(pattern: &Pattern, value: &Value) -> Result<bool, Diagnostic> {
    match pattern {
        Pattern::Name(name) => Err(runtime_error(
            "RUNTIME_UNSUPPORTED_PATTERN",
            format!("name pattern `{name}` is not executable yet"),
        )),
        Pattern::Int(pattern) => {
            let parsed = pattern.parse::<i64>().map_err(|_| {
                runtime_error(
                    "RUNTIME_INVALID_PATTERN",
                    format!("invalid Int match pattern `{pattern}`"),
                )
            })?;
            Ok(matches!(value, Value::Int(value) if *value == parsed))
        }
        Pattern::String(pattern) => Ok(matches!(value, Value::String(value) if value == pattern)),
        Pattern::Bool(pattern) => Ok(matches!(value, Value::Bool(value) if value == pattern)),
        Pattern::Wildcard => Ok(true),
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
            Value::Unit => write!(f, "()"),
        }
    }
}
