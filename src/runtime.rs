use crate::ast::{BinaryOp, Expr, Function, Item, Module, Stmt};
use crate::diagnostic::{Diagnostic, Span};
use crate::mir::{self, MirExpr, MirFunction, MirStmt, Program};
use std::collections::HashMap;
use std::fmt;

const LOOP_LIMIT: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Int(i64),
    String(String),
    Bool(bool),
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
            MirStmt::UnsupportedMatch => Err(runtime_error(
                "RUNTIME_UNSUPPORTED_MATCH",
                "match execution is not implemented in Stage 0",
            )),
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
            MirExpr::Binary { op, left, right } => {
                let left = self.eval_expr(left, locals)?;
                let right = self.eval_expr(right, locals)?;
                eval_binary(*op, left, right)
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
            Stmt::Match { .. } => Err(runtime_error(
                "RUNTIME_UNSUPPORTED_MATCH",
                "match execution is not implemented in Stage 0",
            )),
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
            } => {
                let left = self.eval_expr(left, locals)?;
                let right = self.eval_expr(right, locals)?;
                eval_binary(*op, left, right)
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
        }
    }
}

enum Control {
    Continue,
    Return(Value),
}

fn eval_binary(op: BinaryOp, left: Value, right: Value) -> Result<Value, Diagnostic> {
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
            Value::Unit => write!(f, "()"),
        }
    }
}
