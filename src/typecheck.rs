use crate::ast::{BinaryOp, Expr, Function, Item, Module, Stmt};
use crate::diagnostic::{Diagnostic, Span};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Type {
    Int,
    String,
    Bool,
    Named(String),
    Unit,
}

pub fn check(module: &Module) -> Result<(), Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    for item in &module.items {
        if let Item::Function(function) = item {
            check_function(function, &mut diagnostics);
        }
    }

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

fn check_function(function: &Function, diagnostics: &mut Vec<Diagnostic>) {
    let mut locals = HashMap::new();
    for param in &function.params {
        locals.insert(param.name.clone(), parse_type(&param.ty));
    }
    let return_type = function
        .return_type
        .as_deref()
        .map(parse_type)
        .unwrap_or(Type::Unit);
    check_stmts(&function.body, &mut locals, &return_type, &function.name, diagnostics);
}

fn check_stmts(
    stmts: &[Stmt],
    locals: &mut HashMap<String, Type>,
    return_type: &Type,
    function_name: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for stmt in stmts {
        match stmt {
            Stmt::Let { name, ty, value } => {
                let value_type = infer_expr(value, locals, diagnostics);
                let declared_type = ty.as_deref().map(parse_type);
                if let Some(declared_type) = declared_type {
                    expect_type(&value_type, &declared_type, "TYPE_LET_MISMATCH", diagnostics);
                    locals.insert(name.clone(), declared_type);
                } else {
                    locals.insert(name.clone(), value_type);
                }
            }
            Stmt::If {
                condition,
                then_body,
                else_body,
            } => {
                let condition_type = infer_expr(condition, locals, diagnostics);
                expect_type(&condition_type, &Type::Bool, "TYPE_IF_CONDITION", diagnostics);
                let mut then_locals = locals.clone();
                check_stmts(
                    then_body,
                    &mut then_locals,
                    return_type,
                    function_name,
                    diagnostics,
                );
                let mut else_locals = locals.clone();
                check_stmts(
                    else_body,
                    &mut else_locals,
                    return_type,
                    function_name,
                    diagnostics,
                );
            }
            Stmt::While { condition, body } => {
                let condition_type = infer_expr(condition, locals, diagnostics);
                expect_type(&condition_type, &Type::Bool, "TYPE_WHILE_CONDITION", diagnostics);
                let mut loop_locals = locals.clone();
                check_stmts(
                    body,
                    &mut loop_locals,
                    return_type,
                    function_name,
                    diagnostics,
                );
            }
            Stmt::Match { value, arms } => {
                let _ = infer_expr(value, locals, diagnostics);
                for arm in arms {
                    let mut arm_locals = locals.clone();
                    check_stmts(
                        &arm.body,
                        &mut arm_locals,
                        return_type,
                        function_name,
                        diagnostics,
                    );
                }
            }
            Stmt::Return(Some(value)) => {
                let value_type = infer_expr(value, locals, diagnostics);
                let code = if function_name.is_empty() {
                    "TYPE_RETURN_MISMATCH"
                } else {
                    "TYPE_RETURN_MISMATCH"
                };
                expect_type(&value_type, return_type, code, diagnostics);
            }
            Stmt::Return(None) => {
                expect_type(&Type::Unit, return_type, "TYPE_RETURN_MISMATCH", diagnostics);
            }
            Stmt::Expr(value) => {
                let _ = infer_expr(value, locals, diagnostics);
            }
        }
    }
}

fn infer_expr(
    expr: &Expr,
    locals: &HashMap<String, Type>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Type {
    match expr {
        Expr::Name(name) => locals.get(name).cloned().unwrap_or(Type::Named(name.clone())),
        Expr::Int(_) => Type::Int,
        Expr::String(_) => Type::String,
        Expr::Bool(_) => Type::Bool,
        Expr::Binary { op, left, right } => {
            let left_type = infer_expr(left, locals, diagnostics);
            let right_type = infer_expr(right, locals, diagnostics);
            match op {
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                    expect_type(&left_type, &Type::Int, "TYPE_BINARY_OPERAND", diagnostics);
                    expect_type(&right_type, &Type::Int, "TYPE_BINARY_OPERAND", diagnostics);
                    Type::Int
                }
            }
        }
    }
}

fn parse_type(name: &str) -> Type {
    match name {
        "Int" => Type::Int,
        "String" => Type::String,
        "Bool" => Type::Bool,
        other => Type::Named(other.to_string()),
    }
}

fn expect_type(found: &Type, expected: &Type, code: &'static str, diagnostics: &mut Vec<Diagnostic>) {
    if found == expected {
        return;
    }
    diagnostics.push(Diagnostic::new(
        code,
        format!("expected {}, found {}", expected.display(), found.display()),
        Span::new(0, 0),
    ));
}

impl Type {
    fn display(&self) -> String {
        match self {
            Type::Int => "Int".to_string(),
            Type::String => "String".to_string(),
            Type::Bool => "Bool".to_string(),
            Type::Named(name) => name.clone(),
            Type::Unit => "Unit".to_string(),
        }
    }
}
