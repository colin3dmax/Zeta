use crate::ast::{Expr, Function, Item, Module, Stmt};
use crate::diagnostic::{Diagnostic, Span};
use std::collections::HashSet;

pub fn resolve(module: &Module) -> Result<(), Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    check_top_level(module, &mut diagnostics);
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

fn check_top_level(module: &Module, diagnostics: &mut Vec<Diagnostic>) {
    let mut names = HashSet::new();
    for item in &module.items {
        let Some(name) = item_name(item) else {
            continue;
        };
        if !names.insert(name.to_string()) {
            diagnostics.push(Diagnostic::new(
                "RESOLVE_DUPLICATE_ITEM",
                format!("duplicate top-level item `{name}`"),
                Span::new(0, 0),
            ));
        }
    }
}

fn check_function(function: &Function, diagnostics: &mut Vec<Diagnostic>) {
    let mut locals = HashSet::new();
    for param in &function.params {
        if !locals.insert(param.name.clone()) {
            diagnostics.push(Diagnostic::new(
                "RESOLVE_DUPLICATE_LOCAL",
                format!("duplicate local `{}` in function `{}`", param.name, function.name),
                Span::new(0, 0),
            ));
        }
    }
    check_stmts(&function.body, &mut locals, &function.name, diagnostics);
}

fn check_stmts(
    stmts: &[Stmt],
    locals: &mut HashSet<String>,
    function_name: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for stmt in stmts {
        match stmt {
            Stmt::Let { name, value, .. } => {
                check_expr(value, locals, function_name, diagnostics);
                if !locals.insert(name.clone()) {
                    diagnostics.push(Diagnostic::new(
                        "RESOLVE_DUPLICATE_LOCAL",
                        format!("duplicate local `{name}` in function `{function_name}`"),
                        Span::new(0, 0),
                    ));
                }
            }
            Stmt::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                check_expr(condition, locals, function_name, diagnostics);
                let mut then_locals = locals.clone();
                check_stmts(then_body, &mut then_locals, function_name, diagnostics);
                let mut else_locals = locals.clone();
                check_stmts(else_body, &mut else_locals, function_name, diagnostics);
            }
            Stmt::While { condition, body } => {
                check_expr(condition, locals, function_name, diagnostics);
                let mut loop_locals = locals.clone();
                check_stmts(body, &mut loop_locals, function_name, diagnostics);
            }
            Stmt::Match { value, arms } => {
                check_expr(value, locals, function_name, diagnostics);
                for arm in arms {
                    let mut arm_locals = locals.clone();
                    check_stmts(&arm.body, &mut arm_locals, function_name, diagnostics);
                }
            }
            Stmt::Return(Some(value)) | Stmt::Expr(value) => {
                check_expr(value, locals, function_name, diagnostics);
            }
            Stmt::Return(None) => {}
        }
    }
}

fn check_expr(
    expr: &Expr,
    locals: &HashSet<String>,
    function_name: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match expr {
        Expr::Name(name) => {
            if !locals.contains(name) {
                diagnostics.push(Diagnostic::new(
                    "RESOLVE_UNKNOWN_NAME",
                    format!("unknown name `{name}` in function `{function_name}`"),
                    Span::new(0, 0),
                ));
            }
        }
        Expr::Binary { left, right, .. } => {
            check_expr(left, locals, function_name, diagnostics);
            check_expr(right, locals, function_name, diagnostics);
        }
        Expr::Int(_) | Expr::String(_) | Expr::Bool(_) => {}
    }
}

fn item_name(item: &Item) -> Option<&str> {
    match item {
        Item::Struct(decl) => Some(&decl.name),
        Item::Enum(decl) => Some(&decl.name),
        Item::Function(function) => Some(&function.name),
        Item::ModuleDecl { .. } | Item::Import { .. } => None,
    }
}
