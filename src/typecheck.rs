use crate::ast::{BinaryOp, Expr, Function, Item, Module, Pattern, Stmt, UnaryOp};
use crate::diagnostic::{Diagnostic, Span};
use crate::std_api;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

thread_local! {
    /// All generic type parameter names in the module being checked (functions +
    /// structs + enums). A `Type::Named(P)` for any `P` here is treated as a
    /// wildcard by `expect_type` — instantiation args are erased in this slice,
    /// so generic struct/enum field/payload types check leniently. The
    /// interpreter (run_mir) is the semantic oracle for generic aggregates.
    static TYPE_PARAMS: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

fn set_type_params(module: &Module) {
    let mut all = HashSet::new();
    for item in &module.items {
        match item {
            Item::Function(f) => all.extend(f.type_params.iter().cloned()),
            Item::Struct(s) => all.extend(s.type_params.iter().cloned()),
            Item::Enum(e) => all.extend(e.type_params.iter().cloned()),
            _ => {}
        }
    }
    TYPE_PARAMS.with(|s| *s.borrow_mut() = all);
}

fn is_type_param_type(t: &Type) -> bool {
    matches!(t, Type::Named(n) if TYPE_PARAMS.with(|s| s.borrow().contains(n)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Type {
    Int,
    Float,
    String,
    Bool,
    Array(Box<Type>),
    Tuple(Vec<Type>),
    Fn(Vec<Type>, Box<Type>),
    Range,
    Named(String),
    Unit,
    Error,
}

pub fn check(module: &Module) -> Result<(), Vec<Diagnostic>> {
    let external_functions = standard_external_functions(module);
    let external_enums = standard_external_enums(module);
    check_with_external_items(module, &external_functions, &[], &external_enums)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalFunction {
    pub name: String,
    pub params: Vec<String>,
    pub return_type: Option<String>,
    pub target_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalStruct {
    pub name: String,
    pub fields: Vec<(String, String)>,
    pub target_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalEnum {
    pub name: String,
    /// Generic type parameters (empty for non-generic / monomorphized enums).
    pub type_params: Vec<String>,
    pub variants: Vec<(String, Option<String>)>,
    pub target_name: Option<String>,
}

pub fn check_with_external_functions(
    module: &Module,
    external_functions: &[ExternalFunction],
) -> Result<(), Vec<Diagnostic>> {
    check_with_external_items(module, external_functions, &[], &[])
}

pub fn check_with_external_items(
    module: &Module,
    external_functions: &[ExternalFunction],
    external_structs: &[ExternalStruct],
    external_enums: &[ExternalEnum],
) -> Result<(), Vec<Diagnostic>> {
    set_type_params(module);
    // External (e.g. built-in `Option<T>` / `Result<T, E>`) generic enums also
    // contribute type parameters, so their payload types (`T`/`E`) check
    // leniently as wildcards.
    TYPE_PARAMS.with(|s| {
        let mut set = s.borrow_mut();
        for external_enum in external_enums {
            set.extend(external_enum.type_params.iter().cloned());
        }
    });
    let mut diagnostics = Vec::new();
    let mut structs = struct_types(module);
    for external_struct in external_structs {
        structs
            .entry(external_struct.name.clone())
            .or_insert_with(|| external_struct_type(external_struct));
    }
    let mut enums = enum_types(module);
    for external_enum in external_enums {
        enums
            .entry(external_enum.name.clone())
            .or_insert_with(|| external_enum_type(external_enum));
    }
    let mut functions = function_signatures(module, &structs, &enums);
    for function in external_functions {
        functions
            .entry(function.name.clone())
            .or_insert_with(|| external_function_signature(function, &structs, &enums));
    }
    validate_declared_types(module, &structs, &enums, &mut diagnostics);
    for item in &module.items {
        if let Item::Function(function) = item {
            check_function(function, &functions, &structs, &enums, &mut diagnostics);
        }
    }

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

fn validate_declared_types(
    module: &Module,
    structs: &HashMap<String, StructType>,
    enums: &HashMap<String, EnumType>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for item in &module.items {
        match item {
            Item::Struct(decl) => {
                for field in &decl.fields {
                    validate_type_name(
                        &field.ty,
                        field.ty_span,
                        structs,
                        enums,
                        &decl.type_params,
                        diagnostics,
                    );
                }
            }
            Item::Enum(decl) => {
                for variant in &decl.variants {
                    if let Some(payload_type) = &variant.payload_type {
                        validate_type_name(
                            payload_type,
                            variant.payload_type_span.unwrap_or(variant.name_span),
                            structs,
                            enums,
                            &decl.type_params,
                            diagnostics,
                        );
                    }
                }
            }
            Item::Function(function) => {
                for param in &function.params {
                    validate_type_name(
                        &param.ty,
                        param.ty_span,
                        structs,
                        enums,
                        &function.type_params,
                        diagnostics,
                    );
                }
                if let Some(return_type) = &function.return_type {
                    validate_type_name(
                        return_type,
                        function.return_type_span.unwrap_or(function.name_span),
                        structs,
                        enums,
                        &function.type_params,
                        diagnostics,
                    );
                }
                validate_stmt_types(&function.body, structs, enums, &function.type_params, diagnostics);
            }
            Item::ModuleDecl { .. } | Item::Import { .. } => {}
        }
    }
}

fn validate_stmt_types(
    stmts: &[Stmt],
    structs: &HashMap<String, StructType>,
    enums: &HashMap<String, EnumType>,
    type_params: &[String],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for stmt in stmts {
        match stmt {
            Stmt::Let {
                ty: Some(ty),
                ty_span,
                ..
            } => validate_type_name(
                ty,
                ty_span.expect("typed let should carry a type span"),
                structs,
                enums,
                type_params,
                diagnostics,
            ),
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                validate_stmt_types(then_body, structs, enums, type_params, diagnostics);
                validate_stmt_types(else_body, structs, enums, type_params, diagnostics);
            }
            Stmt::While { body, .. } => {
                validate_stmt_types(body, structs, enums, type_params, diagnostics)
            }
            Stmt::ForIn { body, .. } => {
                validate_stmt_types(body, structs, enums, type_params, diagnostics)
            }
            Stmt::ForC {
                init, step, body, ..
            } => {
                validate_stmt_types(
                    std::slice::from_ref(init.as_ref()),
                    structs,
                    enums,
                    type_params,
                    diagnostics,
                );
                validate_stmt_types(
                    std::slice::from_ref(step.as_ref()),
                    structs,
                    enums,
                    type_params,
                    diagnostics,
                );
                validate_stmt_types(body, structs, enums, type_params, diagnostics);
            }
            Stmt::Match { arms, .. } => {
                for arm in arms {
                    validate_stmt_types(&arm.body, structs, enums, type_params, diagnostics);
                }
            }
            Stmt::Let { ty: None, .. }
            | Stmt::Assign { .. }
            | Stmt::Return(_)
            | Stmt::Break { .. }
            | Stmt::Continue { .. }
            | Stmt::Expr(_) => {}
        }
    }
}

fn validate_type_name(
    name: &str,
    span: Span,
    structs: &HashMap<String, StructType>,
    enums: &HashMap<String, EnumType>,
    type_params: &[String],
    diagnostics: &mut Vec<Diagnostic>,
) {
    if let Some((params, ret)) = crate::type_syntax::fn_parts(name) {
        for part in params {
            validate_type_name(part, span, structs, enums, type_params, diagnostics);
        }
        validate_type_name(ret, span, structs, enums, type_params, diagnostics);
        return;
    }
    if let Some(parts) = crate::type_syntax::tuple_parts(name) {
        for part in parts {
            validate_type_name(part, span, structs, enums, type_params, diagnostics);
        }
        return;
    }
    // Generic instantiation `Box<Int>`: validate the base aggregate name and each
    // argument type. The arguments are otherwise erased for type checking.
    if let Some((base, args)) = crate::type_syntax::generic_parts(name) {
        validate_type_name(base, span, structs, enums, type_params, diagnostics);
        for arg in args {
            validate_type_name(arg, span, structs, enums, type_params, diagnostics);
        }
        return;
    }
    if is_builtin_type_name(name)
        || structs.contains_key(name)
        || enums.contains_key(name)
        || type_params.iter().any(|p| p == name)
    {
        return;
    }
    diagnostics.push(Diagnostic::new(
        "TYPE_UNKNOWN_TYPE",
        format!("unknown type `{name}`"),
        span,
    ));
}

fn is_builtin_type_name(name: &str) -> bool {
    matches!(
        name,
        "Int" | "Float" | "String" | "Bool" | "Unit" | "IntArray" | "StringArray" | "BoolArray"
            | "FloatArray"
    )
}

fn external_function_signature(
    function: &ExternalFunction,
    structs: &HashMap<String, StructType>,
    enums: &HashMap<String, EnumType>,
) -> FunctionSignature {
    FunctionSignature {
        type_params: Vec::new(),
        params: function
            .params
            .iter()
            .map(|param| parse_declared_type(param, structs, enums))
            .collect(),
        return_type: function
            .return_type
            .as_deref()
            .map(|ty| parse_declared_type(ty, structs, enums))
            .unwrap_or(Type::Unit),
    }
}

fn external_struct_type(external_struct: &ExternalStruct) -> StructType {
    StructType {
        fields: external_struct
            .fields
            .iter()
            .map(|(name, ty)| (name.clone(), parse_type(ty)))
            .collect(),
    }
}

fn external_enum_type(external_enum: &ExternalEnum) -> EnumType {
    EnumType {
        variants: external_enum
            .variants
            .iter()
            .map(|(name, payload_type)| (name.clone(), payload_type.as_deref().map(parse_type)))
            .collect(),
    }
}

pub fn standard_external_enums(module: &Module) -> Vec<ExternalEnum> {
    let imports_std_core = module.items.iter().any(|item| match item {
        Item::Import { path, .. } => std_api::is_std_core_import(path),
        _ => false,
    });
    let imports_std_io = module.items.iter().any(|item| match item {
        Item::Import { path, .. } => std_api::is_std_io_import(path),
        _ => false,
    });

    let mut enums = Vec::new();
    if imports_std_core {
        enums.extend(standard_enums("std.core", std_api::core_enums()));
    }
    if imports_std_io {
        enums.extend(standard_enums("std.io", std_api::io_enums()));
    }
    // Generic built-ins (`Option<T>`/`Result<T,E>`) are injected ONLY when the
    // module references them. The monomorphized legacy enums (`OptionInt`, …)
    // stay unconditional. This keeps the MIR/AST of programs that merely
    // `import std.core` without using the generics byte-identical to before —
    // preserving the self-hosting fixpoint parity and golden dumps.
    enums.retain(|e| e.type_params.is_empty() || module_references_type(module, &e.name));
    enums
}

/// Whether the module references the (built-in) aggregate named `name` — via a
/// construction/match `name.Variant`, or any type annotation whose base name is
/// `name` (recursing through generic arguments / tuples / function types).
fn module_references_type(module: &Module, name: &str) -> bool {
    module.items.iter().any(|item| match item {
        Item::Function(f) => {
            f.params.iter().any(|p| ty_refs(&p.ty, name))
                || f.return_type.as_deref().is_some_and(|t| ty_refs(t, name))
                || f.body.iter().any(|s| stmt_refs(s, name))
        }
        Item::Struct(s) => s.fields.iter().any(|fl| ty_refs(&fl.ty, name)),
        Item::Enum(e) => e
            .variants
            .iter()
            .any(|v| v.payload_type.as_deref().is_some_and(|t| ty_refs(t, name))),
        _ => false,
    })
}

/// Whether a type annotation string names the aggregate `name` anywhere — as its
/// base (`Option`, `Option<Int>`) or nested in its arguments (`Box<Option<Int>>`).
fn ty_refs(ty: &str, name: &str) -> bool {
    if crate::type_syntax::base_name(ty) == name {
        return true;
    }
    if let Some((_, args)) = crate::type_syntax::generic_parts(ty) {
        return args.iter().any(|a| ty_refs(a, name));
    }
    if let Some(parts) = crate::type_syntax::tuple_parts(ty) {
        return parts.iter().any(|p| ty_refs(p, name));
    }
    if let Some((params, ret)) = crate::type_syntax::fn_parts(ty) {
        return params.iter().any(|p| ty_refs(p, name)) || ty_refs(ret, name);
    }
    false
}

fn stmt_refs(stmt: &Stmt, name: &str) -> bool {
    match stmt {
        Stmt::Let { ty, value, .. } => {
            ty.as_deref().is_some_and(|t| ty_refs(t, name)) || expr_refs(value, name)
        }
        Stmt::Assign { target, value } => expr_refs(target, name) || expr_refs(value, name),
        Stmt::If {
            condition,
            then_body,
            else_body,
        } => {
            expr_refs(condition, name)
                || then_body.iter().any(|s| stmt_refs(s, name))
                || else_body.iter().any(|s| stmt_refs(s, name))
        }
        Stmt::While { condition, body } => {
            expr_refs(condition, name) || body.iter().any(|s| stmt_refs(s, name))
        }
        Stmt::ForIn { iterable, body, .. } => {
            expr_refs(iterable, name) || body.iter().any(|s| stmt_refs(s, name))
        }
        Stmt::ForC {
            init,
            condition,
            step,
            body,
        } => {
            stmt_refs(init, name)
                || expr_refs(condition, name)
                || stmt_refs(step, name)
                || body.iter().any(|s| stmt_refs(s, name))
        }
        Stmt::Match { value, arms } => {
            expr_refs(value, name)
                || arms.iter().any(|arm| {
                    matches!(&arm.pattern, Pattern::Variant { enum_name, .. } if enum_name == name)
                        || arm.body.iter().any(|s| stmt_refs(s, name))
                })
        }
        Stmt::Return(Some(value)) | Stmt::Expr(value) => expr_refs(value, name),
        Stmt::Return(None) | Stmt::Break { .. } | Stmt::Continue { .. } => false,
    }
}

fn expr_refs(expr: &Expr, name: &str) -> bool {
    match expr {
        Expr::Try { expr, .. } => expr_refs(expr, name),
        Expr::Call { callee, args, .. } => {
            callee.split_once('.').map_or(callee == name, |(base, _)| base == name)
                || args.iter().any(|a| expr_refs(a, name))
        }
        Expr::Binary { left, right, .. } => expr_refs(left, name) || expr_refs(right, name),
        Expr::Unary { expr, .. } => expr_refs(expr, name),
        Expr::Lambda { params, body, .. } => {
            params.iter().any(|p| ty_refs(&p.ty, name)) || expr_refs(body, name)
        }
        Expr::StructLiteral { ty, fields, .. } => {
            ty_refs(ty, name) || fields.iter().any(|f| expr_refs(&f.value, name))
        }
        Expr::FieldAccess { base, .. } => expr_refs(base, name),
        Expr::ArrayLiteral { elements, .. } | Expr::Tuple { elements, .. } => {
            elements.iter().any(|e| expr_refs(e, name))
        }
        Expr::Index { base, index, .. } => expr_refs(base, name) || expr_refs(index, name),
        Expr::Range { start, end, .. } => expr_refs(start, name) || expr_refs(end, name),
        Expr::Name { .. }
        | Expr::Int { .. }
        | Expr::Float { .. }
        | Expr::String { .. }
        | Expr::Bool { .. } => false,
    }
}

pub fn standard_external_functions(module: &Module) -> Vec<ExternalFunction> {
    let imports_std_core = module.items.iter().any(|item| match item {
        Item::Import { path, .. } => std_api::is_std_core_import(path),
        _ => false,
    });
    let imports_std_io = module.items.iter().any(|item| match item {
        Item::Import { path, .. } => std_api::is_std_io_import(path),
        _ => false,
    });

    let mut functions = Vec::new();
    if imports_std_core {
        functions.extend(standard_functions("std.core", std_api::core_functions()));
    }
    if imports_std_io {
        functions.extend(standard_functions("std.io", std_api::io_functions()));
    }
    functions
}

fn standard_enums(module_name: &str, enums: &[std_api::StandardEnum]) -> Vec<ExternalEnum> {
    enums
        .iter()
        .map(|standard_enum| ExternalEnum {
            name: standard_enum.name.to_string(),
            type_params: standard_enum
                .type_params
                .iter()
                .map(|p| p.to_string())
                .collect(),
            variants: standard_enum
                .variants
                .iter()
                .map(|variant| {
                    (
                        variant.name.to_string(),
                        variant.payload_type.map(str::to_string),
                    )
                })
                .collect(),
            target_name: Some(format!("{module_name}.{}", standard_enum.name)),
        })
        .collect()
}

fn standard_functions(
    module_name: &str,
    functions: &[std_api::StandardFunction],
) -> Vec<ExternalFunction> {
    functions
        .iter()
        .map(|function| ExternalFunction {
            name: function.name.to_string(),
            params: function
                .params
                .iter()
                .map(|param| param.to_string())
                .collect(),
            return_type: function.return_type.map(str::to_string),
            target_name: Some(format!("{module_name}.{}", function.name)),
        })
        .collect()
}

fn check_function(
    function: &Function,
    functions: &HashMap<String, FunctionSignature>,
    structs: &HashMap<String, StructType>,
    enums: &HashMap<String, EnumType>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut locals = HashMap::new();
    for param in &function.params {
        locals.insert(
            param.name.clone(),
            Binding {
                ty: parse_type_with_params(&param.ty, structs, enums, &function.type_params),
                mutable: false,
            },
        );
    }
    let return_type = function
        .return_type
        .as_deref()
        .map(|ty| parse_type_with_params(ty, structs, enums, &function.type_params))
        .unwrap_or(Type::Unit);
    check_stmts(
        &function.body,
        &mut locals,
        functions,
        structs,
        enums,
        &return_type,
        &function.name,
        0,
        diagnostics,
    );
}

fn check_stmts(
    stmts: &[Stmt],
    locals: &mut HashMap<String, Binding>,
    functions: &HashMap<String, FunctionSignature>,
    structs: &HashMap<String, StructType>,
    enums: &HashMap<String, EnumType>,
    return_type: &Type,
    function_name: &str,
    loop_depth: usize,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for stmt in stmts {
        match stmt {
            Stmt::Let {
                mutable,
                name,
                ty,
                value,
                ..
            } => {
                let value_type = infer_expr(value, locals, functions, structs, enums, diagnostics);
                let declared_type = ty
                    .as_deref()
                    .map(|ty| parse_declared_type(ty, structs, enums));
                if let Some(declared_type) = declared_type {
                    expect_type(
                        &value_type,
                        &declared_type,
                        "TYPE_LET_MISMATCH",
                        value.span(),
                        diagnostics,
                    );
                    locals.insert(
                        name.clone(),
                        Binding {
                            ty: declared_type,
                            mutable: *mutable,
                        },
                    );
                } else {
                    locals.insert(
                        name.clone(),
                        Binding {
                            ty: value_type,
                            mutable: *mutable,
                        },
                    );
                }
            }
            Stmt::Assign { target, value } => {
                let value_type = infer_expr(value, locals, functions, structs, enums, diagnostics);
                match target {
                    Expr::Name { name, span } => match locals.get(name) {
                        Some(binding) if binding.mutable => {
                            expect_type(
                                &value_type,
                                &binding.ty,
                                "TYPE_ASSIGN_MISMATCH",
                                value.span(),
                                diagnostics,
                            );
                        }
                        Some(_) => diagnostics.push(Diagnostic::new(
                            "TYPE_ASSIGN_IMMUTABLE",
                            format!(
                                "cannot assign to immutable local `{name}`; declare it with `let mut`"
                            ),
                            *span,
                        )),
                        None => diagnostics.push(Diagnostic::new(
                            "TYPE_UNKNOWN_NAME",
                            format!("unknown name `{name}`"),
                            *span,
                        )),
                    },
                    Expr::FieldAccess { .. } | Expr::Index { .. } => {
                        let target_type =
                            infer_expr(target, locals, functions, structs, enums, diagnostics);
                        if let Some((root, root_span)) = target.assign_root() {
                            match locals.get(root) {
                                Some(binding) if binding.mutable => {
                                    expect_type(
                                        &value_type,
                                        &target_type,
                                        "TYPE_ASSIGN_MISMATCH",
                                        value.span(),
                                        diagnostics,
                                    );
                                }
                                Some(_) => diagnostics.push(Diagnostic::new(
                                    "TYPE_ASSIGN_IMMUTABLE",
                                    format!(
                                        "cannot assign to immutable local `{root}`; declare it with `let mut`"
                                    ),
                                    root_span,
                                )),
                                None => {}
                            }
                        }
                    }
                    _ => diagnostics.push(Diagnostic::new(
                        "TYPE_ASSIGN_TARGET",
                        "invalid assignment target".to_string(),
                        target.span(),
                    )),
                }
            }
            Stmt::If {
                condition,
                then_body,
                else_body,
            } => {
                let condition_type =
                    infer_expr(condition, locals, functions, structs, enums, diagnostics);
                expect_type(
                    &condition_type,
                    &Type::Bool,
                    "TYPE_IF_CONDITION",
                    condition.span(),
                    diagnostics,
                );
                let mut then_locals = locals.clone();
                check_stmts(
                    then_body,
                    &mut then_locals,
                    functions,
                    structs,
                    enums,
                    return_type,
                    function_name,
                    loop_depth,
                    diagnostics,
                );
                let mut else_locals = locals.clone();
                check_stmts(
                    else_body,
                    &mut else_locals,
                    functions,
                    structs,
                    enums,
                    return_type,
                    function_name,
                    loop_depth,
                    diagnostics,
                );
            }
            Stmt::While { condition, body } => {
                let condition_type =
                    infer_expr(condition, locals, functions, structs, enums, diagnostics);
                expect_type(
                    &condition_type,
                    &Type::Bool,
                    "TYPE_WHILE_CONDITION",
                    condition.span(),
                    diagnostics,
                );
                let mut loop_locals = locals.clone();
                check_stmts(
                    body,
                    &mut loop_locals,
                    functions,
                    structs,
                    enums,
                    return_type,
                    function_name,
                    loop_depth + 1,
                    diagnostics,
                );
            }
            Stmt::ForIn {
                binding,
                iterable,
                body,
                ..
            } => {
                let iterable_type =
                    infer_expr(iterable, locals, functions, structs, enums, diagnostics);
                let element_type = match iterable_type {
                    Type::Array(element_type) => *element_type,
                    Type::Range => Type::Int,
                    Type::Error => Type::Error,
                    _ => {
                        diagnostics.push(Diagnostic::new(
                            "TYPE_FOR_ITERABLE",
                            "for-in requires an array value",
                            iterable.span(),
                        ));
                        Type::Error
                    }
                };
                let mut loop_locals = locals.clone();
                loop_locals.insert(
                    binding.clone(),
                    Binding {
                        ty: element_type,
                        mutable: false,
                    },
                );
                check_stmts(
                    body,
                    &mut loop_locals,
                    functions,
                    structs,
                    enums,
                    return_type,
                    function_name,
                    loop_depth + 1,
                    diagnostics,
                );
            }
            Stmt::ForC {
                init,
                condition,
                step,
                body,
            } => {
                let mut loop_locals = locals.clone();
                // init declares its binding in loop_locals (scoped to the for).
                check_stmts(
                    std::slice::from_ref(init.as_ref()),
                    &mut loop_locals,
                    functions,
                    structs,
                    enums,
                    return_type,
                    function_name,
                    loop_depth,
                    diagnostics,
                );
                let condition_type = infer_expr(
                    condition,
                    &loop_locals,
                    functions,
                    structs,
                    enums,
                    diagnostics,
                );
                expect_type(
                    &condition_type,
                    &Type::Bool,
                    "TYPE_FORC_CONDITION",
                    condition.span(),
                    diagnostics,
                );
                check_stmts(
                    std::slice::from_ref(step.as_ref()),
                    &mut loop_locals,
                    functions,
                    structs,
                    enums,
                    return_type,
                    function_name,
                    loop_depth + 1,
                    diagnostics,
                );
                check_stmts(
                    body,
                    &mut loop_locals,
                    functions,
                    structs,
                    enums,
                    return_type,
                    function_name,
                    loop_depth + 1,
                    diagnostics,
                );
            }
            Stmt::Match { value, arms } => {
                let value_type = infer_expr(value, locals, functions, structs, enums, diagnostics);
                for arm in arms {
                    let mut arm_locals = locals.clone();
                    for (name, ty) in
                        check_pattern(&arm.pattern, &value_type, value.span(), enums, diagnostics)
                    {
                        arm_locals.insert(name, Binding { ty, mutable: false });
                    }
                    check_stmts(
                        &arm.body,
                        &mut arm_locals,
                        functions,
                        structs,
                        enums,
                        return_type,
                        function_name,
                        loop_depth,
                        diagnostics,
                    );
                }
                check_match_exhaustiveness(&value_type, value.span(), arms, enums, diagnostics);
            }
            Stmt::Return(Some(value)) => {
                let value_type = infer_expr(value, locals, functions, structs, enums, diagnostics);
                let code = if function_name.is_empty() {
                    "TYPE_RETURN_MISMATCH"
                } else {
                    "TYPE_RETURN_MISMATCH"
                };
                expect_type(&value_type, return_type, code, value.span(), diagnostics);
            }
            Stmt::Return(None) => {
                expect_type(
                    &Type::Unit,
                    return_type,
                    "TYPE_RETURN_MISMATCH",
                    Span::new(0, 0),
                    diagnostics,
                );
            }
            Stmt::Break { span } => {
                if loop_depth == 0 {
                    diagnostics.push(Diagnostic::new(
                        "TYPE_BREAK_OUTSIDE_LOOP",
                        "`break` can only be used inside a `while` loop",
                        *span,
                    ));
                }
            }
            Stmt::Continue { span } => {
                if loop_depth == 0 {
                    diagnostics.push(Diagnostic::new(
                        "TYPE_CONTINUE_OUTSIDE_LOOP",
                        "`continue` can only be used inside a `while` loop",
                        *span,
                    ));
                }
            }
            Stmt::Expr(value) => {
                let _ = infer_expr(value, locals, functions, structs, enums, diagnostics);
            }
        }
    }
}

fn check_match_exhaustiveness(
    value_type: &Type,
    value_span: Span,
    arms: &[crate::ast::MatchArm],
    enums: &HashMap<String, EnumType>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if arms
        .iter()
        .any(|arm| matches!(arm.pattern, Pattern::Wildcard | Pattern::Name(_)))
    {
        return;
    }

    match value_type {
        Type::Bool => {
            let covered = arms
                .iter()
                .filter_map(|arm| match arm.pattern {
                    Pattern::Bool(value) => Some(value),
                    _ => None,
                })
                .collect::<HashSet<_>>();
            if !covered.contains(&true) || !covered.contains(&false) {
                diagnostics.push(Diagnostic::new(
                    "TYPE_MATCH_NON_EXHAUSTIVE",
                    "Bool match must cover both true and false or include `_`",
                    value_span,
                ));
            }
        }
        Type::Named(enum_name) => {
            let Some(enum_type) = enums.get(enum_name) else {
                return;
            };
            let covered = arms
                .iter()
                .filter_map(|arm| match &arm.pattern {
                    Pattern::Variant {
                        enum_name: pattern_enum,
                        variant,
                        ..
                    } if pattern_enum == enum_name => Some(variant.as_str()),
                    _ => None,
                })
                .collect::<HashSet<_>>();
            if !enum_type
                .variants
                .keys()
                .all(|variant| covered.contains(variant.as_str()))
            {
                diagnostics.push(Diagnostic::new(
                    "TYPE_MATCH_NON_EXHAUSTIVE",
                    format!("match on `{enum_name}` must cover every variant or include `_`"),
                    value_span,
                ));
            }
        }
        Type::Int
        | Type::Float
        | Type::String
        | Type::Array(_)
        | Type::Tuple(_)
        | Type::Fn(_, _)
        | Type::Range
        | Type::Unit
        | Type::Error => {}
    }
}

fn check_pattern(
    pattern: &Pattern,
    value_type: &Type,
    value_span: Span,
    enums: &HashMap<String, EnumType>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<(String, Type)> {
    match pattern {
        Pattern::Variant {
            enum_name,
            variant,
            binding,
        } => {
            expect_type(
                value_type,
                &Type::Named(enum_name.clone()),
                "TYPE_MATCH_PATTERN",
                value_span,
                diagnostics,
            );
            match enums.get(enum_name) {
                Some(enum_type) => match enum_type.variants.get(variant) {
                    Some(payload_type) => match (payload_type, binding) {
                        (Some(payload_type), Some(binding)) => {
                            return vec![(binding.clone(), payload_type.clone())];
                        }
                        (Some(_), None) => diagnostics.push(Diagnostic::new(
                            "TYPE_ENUM_PATTERN_ARITY",
                            format!(
                                "variant `{enum_name}.{variant}` carries a payload and must bind it"
                            ),
                            value_span,
                        )),
                        (None, Some(_)) => diagnostics.push(Diagnostic::new(
                            "TYPE_ENUM_PATTERN_ARITY",
                            format!("variant `{enum_name}.{variant}` does not carry a payload"),
                            value_span,
                        )),
                        (None, None) => {}
                    },
                    None => diagnostics.push(Diagnostic::new(
                        "TYPE_UNKNOWN_VARIANT",
                        format!("unknown variant `{variant}` on enum `{enum_name}`"),
                        value_span,
                    )),
                },
                None => diagnostics.push(Diagnostic::new(
                    "TYPE_UNKNOWN_ENUM",
                    format!("unknown enum `{enum_name}`"),
                    value_span,
                )),
            }
        }
        Pattern::Int(_) => expect_type(
            value_type,
            &Type::Int,
            "TYPE_MATCH_PATTERN",
            value_span,
            diagnostics,
        ),
        Pattern::String(_) => expect_type(
            value_type,
            &Type::String,
            "TYPE_MATCH_PATTERN",
            value_span,
            diagnostics,
        ),
        Pattern::Bool(_) => expect_type(
            value_type,
            &Type::Bool,
            "TYPE_MATCH_PATTERN",
            value_span,
            diagnostics,
        ),
        Pattern::Name(name) => return vec![(name.clone(), value_type.clone())],
        Pattern::Wildcard => {}
    }
    Vec::new()
}

fn infer_expr(
    expr: &Expr,
    locals: &HashMap<String, Binding>,
    functions: &HashMap<String, FunctionSignature>,
    structs: &HashMap<String, StructType>,
    enums: &HashMap<String, EnumType>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Type {
    match expr {
        Expr::Try { .. } => unreachable!("`?` is desugared before typecheck"),
        Expr::Name { name, .. } => locals
            .get(name)
            .map(|binding| binding.ty.clone())
            .unwrap_or(Type::Error),
        Expr::Int { .. } => Type::Int,
        Expr::Float { .. } => Type::Float,
        Expr::String { .. } => Type::String,
        Expr::Bool { .. } => Type::Bool,
        Expr::Binary {
            op, left, right, ..
        } => {
            let left_type = infer_expr(left, locals, functions, structs, enums, diagnostics);
            let right_type = infer_expr(right, locals, functions, structs, enums, diagnostics);
            match op {
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                    // Numeric: Int or Float (operands must match); result is the
                    // operand type. (Mod / bitwise stay Int-only below.)
                    let numeric = if left_type == Type::Float {
                        Type::Float
                    } else {
                        Type::Int
                    };
                    expect_type(
                        &left_type,
                        &numeric,
                        "TYPE_BINARY_OPERAND",
                        left.span(),
                        diagnostics,
                    );
                    expect_type(
                        &right_type,
                        &numeric,
                        "TYPE_BINARY_OPERAND",
                        right.span(),
                        diagnostics,
                    );
                    numeric
                }
                BinaryOp::Mod | BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
                    expect_type(
                        &left_type,
                        &Type::Int,
                        "TYPE_BINARY_OPERAND",
                        left.span(),
                        diagnostics,
                    );
                    expect_type(
                        &right_type,
                        &Type::Int,
                        "TYPE_BINARY_OPERAND",
                        right.span(),
                        diagnostics,
                    );
                    Type::Int
                }
                BinaryOp::And | BinaryOp::Or => {
                    expect_type(
                        &left_type,
                        &Type::Bool,
                        "TYPE_LOGICAL_OPERAND",
                        left.span(),
                        diagnostics,
                    );
                    expect_type(
                        &right_type,
                        &Type::Bool,
                        "TYPE_LOGICAL_OPERAND",
                        right.span(),
                        diagnostics,
                    );
                    Type::Bool
                }
                BinaryOp::Eq | BinaryOp::NotEq => {
                    expect_type(
                        &right_type,
                        &left_type,
                        "TYPE_EQUALITY_OPERAND",
                        right.span(),
                        diagnostics,
                    );
                    Type::Bool
                }
                BinaryOp::Lt | BinaryOp::Lte | BinaryOp::Gt | BinaryOp::Gte => {
                    let numeric = if left_type == Type::Float {
                        Type::Float
                    } else {
                        Type::Int
                    };
                    expect_type(
                        &left_type,
                        &numeric,
                        "TYPE_ORDERING_OPERAND",
                        left.span(),
                        diagnostics,
                    );
                    expect_type(
                        &right_type,
                        &numeric,
                        "TYPE_ORDERING_OPERAND",
                        right.span(),
                        diagnostics,
                    );
                    Type::Bool
                }
            }
        }
        Expr::Unary { op, expr, .. } => {
            let expr_type = infer_expr(expr, locals, functions, structs, enums, diagnostics);
            match op {
                UnaryOp::Not => {
                    expect_type(
                        &expr_type,
                        &Type::Bool,
                        "TYPE_UNARY_OPERAND",
                        expr.span(),
                        diagnostics,
                    );
                    Type::Bool
                }
                UnaryOp::Neg => {
                    let numeric = if expr_type == Type::Float {
                        Type::Float
                    } else {
                        Type::Int
                    };
                    expect_type(
                        &expr_type,
                        &numeric,
                        "TYPE_UNARY_OPERAND",
                        expr.span(),
                        diagnostics,
                    );
                    numeric
                }
                UnaryOp::BitNot => {
                    expect_type(
                        &expr_type,
                        &Type::Int,
                        "TYPE_UNARY_OPERAND",
                        expr.span(),
                        diagnostics,
                    );
                    Type::Int
                }
            }
        }
        Expr::Call {
            callee,
            callee_span,
            args,
            ..
        } => {
            if let Some((enum_name, variant)) = callee.rsplit_once('.') {
                if let Some(enum_type) = enums.get(enum_name) {
                    return infer_enum_variant_call(
                        enum_name,
                        variant,
                        *callee_span,
                        args,
                        locals,
                        functions,
                        structs,
                        enums,
                        enum_type,
                        diagnostics,
                    );
                }
            }
            let Some(signature) = functions.get(callee) else {
                // Indirect call: the callee may name a local of function type.
                if let Some(binding) = locals.get(callee) {
                    if let Type::Fn(params, ret) = binding.ty.clone() {
                        return infer_indirect_call(
                            callee,
                            &params,
                            &ret,
                            *callee_span,
                            args,
                            locals,
                            functions,
                            structs,
                            enums,
                            diagnostics,
                        );
                    }
                    diagnostics.push(Diagnostic::new(
                        "TYPE_CALL_NOT_CALLABLE",
                        format!("`{callee}` is not a function value"),
                        *callee_span,
                    ));
                }
                return Type::Error;
            };
            if args.len() != signature.params.len() {
                diagnostics.push(Diagnostic::new(
                    "TYPE_CALL_ARITY",
                    format!(
                        "function `{callee}` expects {} arguments, found {}",
                        signature.params.len(),
                        args.len()
                    ),
                    *callee_span,
                ));
                return signature.return_type.clone();
            }
            // For a generic function, infer the type parameters by unifying each
            // declared (generic) param type against the concrete argument type,
            // then substitute into the return type.
            let mut subst: HashMap<String, Type> = HashMap::new();
            for (arg, expected) in args.iter().zip(&signature.params) {
                let found = infer_expr(arg, locals, functions, structs, enums, diagnostics);
                if matches!(found, Type::Error) {
                    continue;
                }
                if signature.type_params.is_empty() {
                    expect_type(&found, expected, "TYPE_CALL_ARGUMENT", arg.span(), diagnostics);
                } else if !unify_generic(expected, &found, &signature.type_params, &mut subst) {
                    expect_type(
                        &found,
                        &substitute_generic(expected, &subst),
                        "TYPE_CALL_ARGUMENT",
                        arg.span(),
                        diagnostics,
                    );
                }
            }
            if signature.type_params.is_empty() {
                signature.return_type.clone()
            } else {
                substitute_generic(&signature.return_type, &subst)
            }
        }
        Expr::Lambda { params, body, .. } => {
            // Infer the body with the lambda's typed params added to the
            // enclosing scope (captures resolve against the outer locals).
            let mut body_locals = locals.clone();
            let mut param_types = Vec::with_capacity(params.len());
            for param in params {
                let ty = parse_type(&param.ty);
                body_locals.insert(
                    param.name.clone(),
                    Binding {
                        ty: ty.clone(),
                        mutable: false,
                    },
                );
                param_types.push(ty);
            }
            let ret = infer_expr(body, &body_locals, functions, structs, enums, diagnostics);
            Type::Fn(param_types, Box::new(ret))
        }
        Expr::StructLiteral {
            ty,
            ty_span,
            fields,
            ..
        } => {
            let Some(struct_type) = structs.get(ty) else {
                diagnostics.push(Diagnostic::new(
                    "TYPE_UNKNOWN_STRUCT",
                    format!("unknown struct `{ty}`"),
                    *ty_span,
                ));
                for field in fields {
                    let _ =
                        infer_expr(&field.value, locals, functions, structs, enums, diagnostics);
                }
                return Type::Named(ty.clone());
            };

            let mut seen = HashMap::new();
            for field in fields {
                if seen.insert(field.name.clone(), field.name_span).is_some() {
                    diagnostics.push(Diagnostic::new(
                        "TYPE_DUPLICATE_FIELD",
                        format!("duplicate field `{}` in `{ty}` literal", field.name),
                        field.name_span,
                    ));
                }
                let found =
                    infer_expr(&field.value, locals, functions, structs, enums, diagnostics);
                match struct_type.fields.get(&field.name) {
                    Some(expected) => {
                        expect_type(
                            &found,
                            expected,
                            "TYPE_STRUCT_FIELD",
                            field.value.span(),
                            diagnostics,
                        );
                    }
                    None => diagnostics.push(Diagnostic::new(
                        "TYPE_UNKNOWN_FIELD",
                        format!("unknown field `{}` on struct `{ty}`", field.name),
                        field.name_span,
                    )),
                }
            }
            for field in struct_type.fields.keys() {
                if !seen.contains_key(field) {
                    diagnostics.push(Diagnostic::new(
                        "TYPE_MISSING_FIELD",
                        format!("missing field `{field}` in `{ty}` literal"),
                        *ty_span,
                    ));
                }
            }
            Type::Named(ty.clone())
        }
        Expr::ArrayLiteral { elements, span } => {
            let Some((first, rest)) = elements.split_first() else {
                diagnostics.push(Diagnostic::new(
                    "TYPE_ARRAY_EMPTY",
                    "empty array literals need an explicit element type in Stage 0",
                    *span,
                ));
                return Type::Error;
            };
            let element_type = infer_expr(first, locals, functions, structs, enums, diagnostics);
            for element in rest {
                let found = infer_expr(element, locals, functions, structs, enums, diagnostics);
                expect_type(
                    &found,
                    &element_type,
                    "TYPE_ARRAY_ELEMENT",
                    element.span(),
                    diagnostics,
                );
            }
            if matches!(element_type, Type::Error) {
                Type::Error
            } else {
                Type::Array(Box::new(element_type))
            }
        }
        Expr::Tuple { elements, .. } => {
            let types: Vec<Type> = elements
                .iter()
                .map(|element| infer_expr(element, locals, functions, structs, enums, diagnostics))
                .collect();
            if types.iter().any(|t| matches!(t, Type::Error)) {
                Type::Error
            } else {
                Type::Tuple(types)
            }
        }
        Expr::Index { base, index, .. } => {
            let base_type = infer_expr(base, locals, functions, structs, enums, diagnostics);
            let index_type = infer_expr(index, locals, functions, structs, enums, diagnostics);
            expect_type(
                &index_type,
                &Type::Int,
                "TYPE_INDEX",
                index.span(),
                diagnostics,
            );
            match base_type {
                Type::Array(element_type) => *element_type,
                Type::Error => Type::Error,
                _ => {
                    diagnostics.push(Diagnostic::new(
                        "TYPE_INDEX_BASE",
                        "index expression requires an array value",
                        base.span(),
                    ));
                    Type::Error
                }
            }
        }
        Expr::Range { start, end, .. } => {
            let start_type = infer_expr(start, locals, functions, structs, enums, diagnostics);
            expect_type(
                &start_type,
                &Type::Int,
                "TYPE_RANGE_BOUND",
                start.span(),
                diagnostics,
            );
            let end_type = infer_expr(end, locals, functions, structs, enums, diagnostics);
            expect_type(
                &end_type,
                &Type::Int,
                "TYPE_RANGE_BOUND",
                end.span(),
                diagnostics,
            );
            Type::Range
        }
        Expr::FieldAccess {
            base,
            field,
            field_span,
            ..
        } => {
            if let Expr::Name {
                name: enum_name, ..
            } = base.as_ref()
            {
                if !locals.contains_key(enum_name) {
                    match enums.get(enum_name) {
                        Some(enum_type) => {
                            if let Some(payload_type) = enum_type.variants.get(field) {
                                if payload_type.is_some() {
                                    diagnostics.push(Diagnostic::new(
                                        "TYPE_ENUM_VARIANT_ARITY",
                                        format!(
                                            "variant `{enum_name}.{field}` carries a payload and must be called"
                                        ),
                                        *field_span,
                                    ));
                                }
                                return Type::Named(enum_name.clone());
                            } else {
                                diagnostics.push(Diagnostic::new(
                                    "TYPE_UNKNOWN_VARIANT",
                                    format!("unknown variant `{field}` on enum `{enum_name}`"),
                                    *field_span,
                                ));
                                return Type::Named(enum_name.clone());
                            }
                        }
                        None => {}
                    }
                }
            }
            let base_type = infer_expr(base, locals, functions, structs, enums, diagnostics);
            if let Type::Array(_) = &base_type {
                if field == "len" {
                    return Type::Int;
                }
                diagnostics.push(Diagnostic::new(
                    "TYPE_ARRAY_FIELD",
                    format!("unknown field `{field}` on array; only `len` is supported"),
                    *field_span,
                ));
                return Type::Error;
            }
            if let Type::Tuple(elements) = &base_type {
                match field.parse::<usize>() {
                    Ok(index) if index < elements.len() => return elements[index].clone(),
                    _ => {
                        diagnostics.push(Diagnostic::new(
                            "TYPE_TUPLE_INDEX",
                            format!(
                                "tuple index `.{field}` is out of range for a {}-element tuple",
                                elements.len()
                            ),
                            *field_span,
                        ));
                        return Type::Error;
                    }
                }
            }
            let Type::Named(struct_name) = base_type else {
                diagnostics.push(Diagnostic::new(
                    "TYPE_FIELD_BASE",
                    "field access requires a struct value",
                    base.span(),
                ));
                return Type::Named(field.clone());
            };
            let Some(struct_type) = structs.get(&struct_name) else {
                diagnostics.push(Diagnostic::new(
                    "TYPE_UNKNOWN_STRUCT",
                    format!("unknown struct `{struct_name}`"),
                    base.span(),
                ));
                return Type::Named(field.clone());
            };
            struct_type.fields.get(field).cloned().unwrap_or_else(|| {
                diagnostics.push(Diagnostic::new(
                    "TYPE_UNKNOWN_FIELD",
                    format!("unknown field `{field}` on struct `{struct_name}`"),
                    *field_span,
                ));
                Type::Named(field.clone())
            })
        }
    }
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_arguments)]
fn infer_indirect_call(
    callee: &str,
    params: &[Type],
    ret: &Type,
    callee_span: Span,
    args: &[Expr],
    locals: &HashMap<String, Binding>,
    functions: &HashMap<String, FunctionSignature>,
    structs: &HashMap<String, StructType>,
    enums: &HashMap<String, EnumType>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Type {
    if args.len() != params.len() {
        diagnostics.push(Diagnostic::new(
            "TYPE_CALL_ARITY",
            format!(
                "closure `{callee}` expects {} arguments, found {}",
                params.len(),
                args.len()
            ),
            callee_span,
        ));
        return ret.clone();
    }
    for (arg, expected) in args.iter().zip(params) {
        let found = infer_expr(arg, locals, functions, structs, enums, diagnostics);
        expect_type(&found, expected, "TYPE_CALL_ARGUMENT", arg.span(), diagnostics);
    }
    ret.clone()
}

fn infer_enum_variant_call(
    enum_name: &str,
    variant: &str,
    callee_span: Span,
    args: &[Expr],
    locals: &HashMap<String, Binding>,
    functions: &HashMap<String, FunctionSignature>,
    structs: &HashMap<String, StructType>,
    enums: &HashMap<String, EnumType>,
    enum_type: &EnumType,
    diagnostics: &mut Vec<Diagnostic>,
) -> Type {
    match enum_type.variants.get(variant) {
        Some(Some(payload_type)) => {
            if args.len() != 1 {
                diagnostics.push(Diagnostic::new(
                    "TYPE_ENUM_VARIANT_ARITY",
                    format!(
                        "variant `{enum_name}.{variant}` expects 1 payload argument, found {}",
                        args.len()
                    ),
                    callee_span,
                ));
            }
            for arg in args {
                let found = infer_expr(arg, locals, functions, structs, enums, diagnostics);
                expect_type(
                    &found,
                    payload_type,
                    "TYPE_ENUM_VARIANT_PAYLOAD",
                    arg.span(),
                    diagnostics,
                );
            }
        }
        Some(None) => {
            if !args.is_empty() {
                diagnostics.push(Diagnostic::new(
                    "TYPE_ENUM_VARIANT_ARITY",
                    format!(
                        "variant `{enum_name}.{variant}` expects no payload arguments, found {}",
                        args.len()
                    ),
                    callee_span,
                ));
            }
            for arg in args {
                let _ = infer_expr(arg, locals, functions, structs, enums, diagnostics);
            }
        }
        None => diagnostics.push(Diagnostic::new(
            "TYPE_UNKNOWN_VARIANT",
            format!("unknown variant `{variant}` on enum `{enum_name}`"),
            callee_span,
        )),
    }
    Type::Named(enum_name.to_string())
}

#[derive(Debug, Clone)]
struct FunctionSignature {
    /// Generic type parameters (empty for a non-generic function). When present,
    /// `params`/`return_type` may contain `Type::Named(P)` for each `P` here.
    type_params: Vec<String>,
    params: Vec<Type>,
    return_type: Type,
}

/// Like [`parse_declared_type`] but treats any name in `type_params` as an opaque
/// generic type `Type::Named(name)` (recursing through tuple/function types).
fn parse_type_with_params(
    name: &str,
    structs: &HashMap<String, StructType>,
    enums: &HashMap<String, EnumType>,
    type_params: &[String],
) -> Type {
    if let Some((params, ret)) = crate::type_syntax::fn_parts(name) {
        return Type::Fn(
            params
                .iter()
                .map(|p| parse_type_with_params(p, structs, enums, type_params))
                .collect(),
            Box::new(parse_type_with_params(ret, structs, enums, type_params)),
        );
    }
    if let Some(parts) = crate::type_syntax::tuple_parts(name) {
        return Type::Tuple(
            parts
                .iter()
                .map(|p| parse_type_with_params(p, structs, enums, type_params))
                .collect(),
        );
    }
    if type_params.iter().any(|p| p == name) {
        return Type::Named(name.to_string());
    }
    parse_declared_type(name, structs, enums)
}

/// Bind generic type parameters by structurally unifying a (possibly generic)
/// declared type against a concrete argument type. Records `P -> concrete` into
/// `subst`. Returns false on a structural mismatch.
fn unify_generic(declared: &Type, actual: &Type, type_params: &[String], subst: &mut HashMap<String, Type>) -> bool {
    if let Type::Named(name) = declared {
        if type_params.iter().any(|p| p == name) {
            return match subst.get(name) {
                Some(bound) => bound == actual,
                None => {
                    subst.insert(name.clone(), actual.clone());
                    true
                }
            };
        }
    }
    match (declared, actual) {
        (Type::Array(d), Type::Array(a)) => unify_generic(d, a, type_params, subst),
        (Type::Tuple(ds), Type::Tuple(as_)) => {
            ds.len() == as_.len()
                && ds
                    .iter()
                    .zip(as_)
                    .all(|(d, a)| unify_generic(d, a, type_params, subst))
        }
        (Type::Fn(dp, dr), Type::Fn(ap, ar)) => {
            dp.len() == ap.len()
                && dp
                    .iter()
                    .zip(ap)
                    .all(|(d, a)| unify_generic(d, a, type_params, subst))
                && unify_generic(dr, ar, type_params, subst)
        }
        _ => declared == actual,
    }
}

/// Substitute bound generic parameters into a type.
fn substitute_generic(ty: &Type, subst: &HashMap<String, Type>) -> Type {
    match ty {
        Type::Named(name) => subst.get(name).cloned().unwrap_or_else(|| ty.clone()),
        Type::Array(inner) => Type::Array(Box::new(substitute_generic(inner, subst))),
        Type::Tuple(elems) => {
            Type::Tuple(elems.iter().map(|e| substitute_generic(e, subst)).collect())
        }
        Type::Fn(params, ret) => Type::Fn(
            params.iter().map(|p| substitute_generic(p, subst)).collect(),
            Box::new(substitute_generic(ret, subst)),
        ),
        _ => ty.clone(),
    }
}

#[derive(Debug, Clone)]
struct StructType {
    fields: HashMap<String, Type>,
}

#[derive(Debug, Clone)]
struct EnumType {
    variants: HashMap<String, Option<Type>>,
}

#[derive(Debug, Clone)]
struct Binding {
    ty: Type,
    mutable: bool,
}

fn function_signatures(
    module: &Module,
    structs: &HashMap<String, StructType>,
    enums: &HashMap<String, EnumType>,
) -> HashMap<String, FunctionSignature> {
    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(function) => Some((
                function.name.clone(),
                FunctionSignature {
                    type_params: function.type_params.clone(),
                    params: function
                        .params
                        .iter()
                        .map(|param| {
                            parse_type_with_params(
                                &param.ty,
                                structs,
                                enums,
                                &function.type_params,
                            )
                        })
                        .collect(),
                    return_type: function
                        .return_type
                        .as_deref()
                        .map(|ty| {
                            parse_type_with_params(ty, structs, enums, &function.type_params)
                        })
                        .unwrap_or(Type::Unit),
                },
            )),
            _ => None,
        })
        .collect()
}

fn struct_types(module: &Module) -> HashMap<String, StructType> {
    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(decl) => Some((
                decl.name.clone(),
                StructType {
                    fields: decl
                        .fields
                        .iter()
                        .map(|field| (field.name.clone(), parse_type(&field.ty)))
                        .collect(),
                },
            )),
            _ => None,
        })
        .collect()
}

fn enum_types(module: &Module) -> HashMap<String, EnumType> {
    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Enum(decl) => Some((
                decl.name.clone(),
                EnumType {
                    variants: decl
                        .variants
                        .iter()
                        .map(|variant| {
                            (
                                variant.name.clone(),
                                variant.payload_type.as_deref().map(parse_type),
                            )
                        })
                        .collect(),
                },
            )),
            _ => None,
        })
        .collect()
}

fn parse_type(name: &str) -> Type {
    if let Some((params, ret)) = crate::type_syntax::fn_parts(name) {
        return Type::Fn(
            params.iter().map(|p| parse_type(p)).collect(),
            Box::new(parse_type(ret)),
        );
    }
    if let Some(parts) = crate::type_syntax::tuple_parts(name) {
        return Type::Tuple(parts.iter().map(|p| parse_type(p)).collect());
    }
    // Generic instantiation `Box<Int>` is erased to its base name for type
    // checking (the interpreter is the semantic oracle for generic aggregates).
    if let Some((base, _)) = crate::type_syntax::generic_parts(name) {
        return parse_type(base);
    }
    match name {
        "Int" => Type::Int,
        "Float" => Type::Float,
        "String" => Type::String,
        "Bool" => Type::Bool,
        "IntArray" => Type::Array(Box::new(Type::Int)),
        "StringArray" => Type::Array(Box::new(Type::String)),
        "BoolArray" => Type::Array(Box::new(Type::Bool)),
        "FloatArray" => Type::Array(Box::new(Type::Float)),
        "Unit" => Type::Unit,
        other => Type::Named(other.to_string()),
    }
}

fn parse_declared_type(
    name: &str,
    structs: &HashMap<String, StructType>,
    enums: &HashMap<String, EnumType>,
) -> Type {
    if let Some((params, ret)) = crate::type_syntax::fn_parts(name) {
        return Type::Fn(
            params
                .iter()
                .map(|p| parse_declared_type(p, structs, enums))
                .collect(),
            Box::new(parse_declared_type(ret, structs, enums)),
        );
    }
    if let Some(parts) = crate::type_syntax::tuple_parts(name) {
        return Type::Tuple(
            parts
                .iter()
                .map(|p| parse_declared_type(p, structs, enums))
                .collect(),
        );
    }
    // Generic instantiation `Box<Int>` is validated/erased by its base name.
    if let Some((base, _)) = crate::type_syntax::generic_parts(name) {
        return parse_declared_type(base, structs, enums);
    }
    if is_builtin_type_name(name) || structs.contains_key(name) || enums.contains_key(name) {
        parse_type(name)
    } else {
        Type::Error
    }
}

fn expect_type(
    found: &Type,
    expected: &Type,
    code: &'static str,
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if found == expected {
        return;
    }
    if matches!(found, Type::Error) || matches!(expected, Type::Error) {
        return;
    }
    // A generic type parameter (erased instantiation) is compatible with any
    // concrete type for assignment-like checks (let/return/call-arg/field/
    // payload/match). But an OPERATION that requires a concrete capability
    // (arithmetic, ordering, logical, index, condition) must still reject an
    // unconstrained type parameter — there are no trait bounds.
    let operand_constraint = matches!(
        code,
        "TYPE_BINARY_OPERAND"
            | "TYPE_LOGICAL_OPERAND"
            | "TYPE_ORDERING_OPERAND"
            | "TYPE_UNARY_OPERAND"
            | "TYPE_INDEX"
            | "TYPE_RANGE_BOUND"
            | "TYPE_IF_CONDITION"
            | "TYPE_WHILE_CONDITION"
            | "TYPE_FORC_CONDITION"
    );
    if !operand_constraint && (is_type_param_type(found) || is_type_param_type(expected)) {
        return;
    }
    diagnostics.push(Diagnostic::new(
        code,
        format!("expected {}, found {}", expected.display(), found.display()),
        span,
    ));
}

impl Type {
    fn display(&self) -> String {
        match self {
            Type::Int => "Int".to_string(),
            Type::Float => "Float".to_string(),
            Type::String => "String".to_string(),
            Type::Bool => "Bool".to_string(),
            Type::Array(element) => match element.as_ref() {
                Type::Int => "IntArray".to_string(),
                Type::String => "StringArray".to_string(),
                Type::Bool => "BoolArray".to_string(),
                Type::Float => "FloatArray".to_string(),
                other => format!("{}Array", other.display()),
            },
            Type::Tuple(elements) => {
                let inner: Vec<String> = elements.iter().map(|e| e.display()).collect();
                format!("({})", inner.join(", "))
            }
            Type::Fn(params, ret) => {
                let inner: Vec<String> = params.iter().map(|p| p.display()).collect();
                format!("fn({}) -> {}", inner.join(", "), ret.display())
            }
            Type::Range => "Range".to_string(),
            Type::Named(name) => name.clone(),
            Type::Unit => "Unit".to_string(),
            Type::Error => "<error>".to_string(),
        }
    }
}
