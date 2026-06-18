//! Pre-resolve desugaring of the postfix `?` (try) operator.
//!
//! `expr?` unwraps an `Option`/`Result` operand, early-returning the failure
//! variant (`None` / `Err(e)`) from the enclosing function. Because this
//! language has no match-expression and no uninitialized `let`, `?` is lowered
//! by moving the *continuation* (the rest of the block) into the success arm of
//! a `match`:
//!
//! ```text
//! let n = expr?;            match expr {
//! <rest>            ──►       Result.Ok(n)   -> { <rest> }
//!                            Result.Err(e0) -> { return Result.Err(e0); }
//!                          }
//! ```
//!
//! Dispatch (`Ok`/`Err` vs `Some`/`None`) is by the enclosing function's return
//! type. The pass fully eliminates [`Expr::Try`], so resolve/typecheck/mir never
//! see it. It reuses the ordinary `match` + enum + early-return machinery, so no
//! new MIR or codegen support is needed.

use crate::ast::*;
use crate::diagnostic::{Diagnostic, Span};

#[derive(Clone, Copy, PartialEq, Eq)]
enum TryKind {
    Result,
    Option,
}

/// Rewrite every `?` in the module into a `match`. Errors if `?` is used in a
/// function that does not return `Option`/`Result` (the only forms the operator
/// dispatches on).
pub fn desugar_try(module: &mut Module) -> Result<(), Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    for item in &mut module.items {
        if let Item::Function(function) = item {
            let kind = function
                .return_type
                .as_deref()
                .and_then(try_kind_of_return);
            let mut counter = 0usize;
            let body = std::mem::take(&mut function.body);
            function.body = desugar_block(body, kind, &mut counter, &mut diagnostics);
        }
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

fn try_kind_of_return(return_type: &str) -> Option<TryKind> {
    match crate::type_syntax::base_name(return_type) {
        "Result" => Some(TryKind::Result),
        "Option" => Some(TryKind::Option),
        _ => None,
    }
}

/// Desugar a statement block. When a statement carries an immediate `?`, the
/// statement (with the `?` replaced by a fresh binding) and every statement
/// after it move into the success arm of a generated `match`.
fn desugar_block(
    stmts: Vec<Stmt>,
    kind: Option<TryKind>,
    counter: &mut usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Stmt> {
    let mut out = Vec::new();
    let mut stmts = stmts.into_iter();
    while let Some(mut stmt) = stmts.next() {
        if let Some(extracted) = take_immediate_try(&mut stmt, counter) {
            // Continuation = this statement (its `?` now a binding) + the rest.
            let mut rest = vec![stmt];
            rest.extend(stmts);
            let match_stmt = build_try_match(extracted, rest, kind, counter, diagnostics);
            out.extend(desugar_block(vec![match_stmt], kind, counter, diagnostics));
            return out;
        }
        out.push(desugar_nested(stmt, kind, counter, diagnostics));
    }
    out
}

/// A `?` extracted from a statement: its operand, the fresh name now standing in
/// for the unwrapped success value, and the operator span (for diagnostics).
struct Extracted {
    operand: Expr,
    value_name: String,
    span: Span,
}

/// Find the left-most `?` among a statement's *immediately evaluated*
/// expressions (not the bodies of nested blocks, which `desugar_nested`
/// handles), replacing it in place with a reference to a fresh binding.
fn take_immediate_try(stmt: &mut Stmt, counter: &mut usize) -> Option<Extracted> {
    match stmt {
        Stmt::Let { value, .. } => extract_try(value, counter),
        Stmt::Assign { target, value } => {
            extract_try(target, counter).or_else(|| extract_try(value, counter))
        }
        Stmt::Return(Some(value)) | Stmt::Expr(value) => extract_try(value, counter),
        Stmt::If { condition, .. } => extract_try(condition, counter),
        Stmt::Match { value, .. } => extract_try(value, counter),
        Stmt::ForIn { iterable, .. } => extract_try(iterable, counter),
        // Loop conditions/steps are evaluated repeatedly, so hoisting their
        // continuation would be unsound; `?` there is left for typecheck to
        // reject (it never appears in practice). Bodies are handled by
        // `desugar_nested`.
        _ => None,
    }
}

/// Replace the left-most `?` in `expr` (in evaluation order, not descending into
/// lambda bodies) with a fresh `Name`, returning what was unwrapped.
fn extract_try(expr: &mut Expr, counter: &mut usize) -> Option<Extracted> {
    match expr {
        Expr::Try { span, .. } => {
            let span = *span;
            let value_name = fresh(counter, "val");
            // Replace the whole `e?` node with the binding name; recover `e`.
            let placeholder = Expr::Name {
                name: value_name.clone(),
                span,
            };
            let Expr::Try { expr: inner, .. } = std::mem::replace(expr, placeholder) else {
                unreachable!("matched Try above");
            };
            Some(Extracted {
                operand: *inner,
                value_name,
                span,
            })
        }
        Expr::Binary { left, right, .. } => {
            extract_try(left, counter).or_else(|| extract_try(right, counter))
        }
        Expr::Unary { expr, .. } => extract_try(expr, counter),
        Expr::Call { args, .. } => args.iter_mut().find_map(|a| extract_try(a, counter)),
        Expr::StructLiteral { fields, .. } => {
            fields.iter_mut().find_map(|f| extract_try(&mut f.value, counter))
        }
        Expr::FieldAccess { base, .. } => extract_try(base, counter),
        Expr::ArrayLiteral { elements, .. } | Expr::Tuple { elements, .. } => {
            elements.iter_mut().find_map(|e| extract_try(e, counter))
        }
        Expr::Index { base, index, .. } => {
            extract_try(base, counter).or_else(|| extract_try(index, counter))
        }
        Expr::Range { start, end, .. } => {
            extract_try(start, counter).or_else(|| extract_try(end, counter))
        }
        // A `?` inside a lambda body belongs to the lambda, not this function;
        // leave it (lambdas can't early-return the outer function anyway).
        Expr::Lambda { .. }
        | Expr::Name { .. }
        | Expr::Int { .. }
        | Expr::Float { .. }
        | Expr::String { .. }
        | Expr::Bool { .. } => None,
    }
}

/// Recurse into a statement's nested block bodies (each its own scope), leaving
/// the statement's immediate expressions untouched.
fn desugar_nested(
    stmt: Stmt,
    kind: Option<TryKind>,
    counter: &mut usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> Stmt {
    match stmt {
        Stmt::If {
            condition,
            then_body,
            else_body,
        } => Stmt::If {
            condition,
            then_body: desugar_block(then_body, kind, counter, diagnostics),
            else_body: desugar_block(else_body, kind, counter, diagnostics),
        },
        Stmt::While { condition, body } => Stmt::While {
            condition,
            body: desugar_block(body, kind, counter, diagnostics),
        },
        Stmt::ForIn {
            binding,
            binding_span,
            iterable,
            body,
        } => Stmt::ForIn {
            binding,
            binding_span,
            iterable,
            body: desugar_block(body, kind, counter, diagnostics),
        },
        Stmt::ForC {
            init,
            condition,
            step,
            body,
        } => Stmt::ForC {
            init,
            condition,
            step,
            body: desugar_block(body, kind, counter, diagnostics),
        },
        Stmt::Match { value, arms } => Stmt::Match {
            value,
            arms: arms
                .into_iter()
                .map(|arm| MatchArm {
                    pattern: arm.pattern,
                    body: desugar_block(arm.body, kind, counter, diagnostics),
                })
                .collect(),
        },
        other => other,
    }
}

/// Build the `match operand { <ok> -> { <ok_body> }, <fail> -> { return <fail> } }`
/// that a single `?` desugars to.
fn build_try_match(
    extracted: Extracted,
    ok_body: Vec<Stmt>,
    kind: Option<TryKind>,
    counter: &mut usize,
    diagnostics: &mut Vec<Diagnostic>,
) -> Stmt {
    let Extracted {
        operand,
        value_name,
        span,
    } = extracted;
    let kind = match kind {
        Some(kind) => kind,
        None => {
            diagnostics.push(Diagnostic::new(
                "DESUGAR_TRY_OUTSIDE_RESULT_OPTION",
                "`?` can only be used in a function returning `Option` or `Result`".to_string(),
                span,
            ));
            // Degrade to a Result shape so lowering can proceed to surface the
            // error; compilation already fails on the diagnostic above.
            TryKind::Result
        }
    };
    let (enum_name, ok_variant, fail_variant) = match kind {
        TryKind::Result => ("Result", "Ok", "Err"),
        TryKind::Option => ("Option", "Some", "None"),
    };
    let fail_arm = match kind {
        TryKind::Result => {
            let err_name = fresh(counter, "err");
            MatchArm {
                pattern: Pattern::Variant {
                    enum_name: enum_name.to_string(),
                    variant: fail_variant.to_string(),
                    binding: Some(err_name.clone()),
                },
                // return Result.Err(err)
                body: vec![Stmt::Return(Some(Expr::Call {
                    callee: format!("{enum_name}.{fail_variant}"),
                    callee_span: span,
                    args: vec![name_expr(&err_name, span)],
                    span,
                }))],
            }
        }
        TryKind::Option => MatchArm {
            pattern: Pattern::Variant {
                enum_name: enum_name.to_string(),
                variant: fail_variant.to_string(),
                binding: None,
            },
            // return Option.None  (no payload — a field-access form)
            body: vec![Stmt::Return(Some(Expr::FieldAccess {
                base: Box::new(name_expr(enum_name, span)),
                field: fail_variant.to_string(),
                field_span: span,
                span,
            }))],
        },
    };
    let ok_arm = MatchArm {
        pattern: Pattern::Variant {
            enum_name: enum_name.to_string(),
            variant: ok_variant.to_string(),
            binding: Some(value_name),
        },
        body: ok_body,
    };
    Stmt::Match {
        value: operand,
        arms: vec![ok_arm, fail_arm],
    }
}

fn name_expr(name: &str, span: Span) -> Expr {
    Expr::Name {
        name: name.to_string(),
        span,
    }
}

fn fresh(counter: &mut usize, role: &str) -> String {
    let n = *counter;
    *counter += 1;
    format!("__try_{role}{n}")
}
