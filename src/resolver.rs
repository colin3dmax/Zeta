use crate::ast::{Expr, Function, Item, Module, Pattern, Stmt};
use crate::diagnostic::{Diagnostic, Span};
use crate::std_api;
use std::collections::{HashMap, HashSet};

pub fn resolve(module: &Module) -> Result<(), Vec<Diagnostic>> {
    resolve_with_imports_and_functions(module, &HashSet::new(), &HashSet::new())
}

pub fn resolve_with_imports(
    module: &Module,
    local_imports: &HashSet<String>,
) -> Result<(), Vec<Diagnostic>> {
    resolve_with_imports_and_functions(module, local_imports, &HashSet::new())
}

pub fn resolve_with_imports_and_functions(
    module: &Module,
    local_imports: &HashSet<String>,
    external_functions: &HashSet<String>,
) -> Result<(), Vec<Diagnostic>> {
    resolve_with_imports_functions_and_ambiguous(
        module,
        local_imports,
        external_functions,
        &HashSet::new(),
    )
}

pub fn resolve_with_imports_functions_and_ambiguous(
    module: &Module,
    local_imports: &HashSet<String>,
    external_functions: &HashSet<String>,
    ambiguous_external_functions: &HashSet<String>,
) -> Result<(), Vec<Diagnostic>> {
    resolve_with_imports_functions_enums_and_ambiguous(
        module,
        local_imports,
        external_functions,
        &HashMap::new(),
        ambiguous_external_functions,
    )
}

pub fn resolve_with_imports_functions_enums_and_ambiguous(
    module: &Module,
    local_imports: &HashSet<String>,
    external_functions: &HashSet<String>,
    external_enum_variants: &HashMap<String, HashSet<String>>,
    ambiguous_external_functions: &HashSet<String>,
) -> Result<(), Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    check_top_level(module, local_imports, &mut diagnostics);
    let mut functions = function_names(module);
    functions.extend(external_functions.iter().cloned());
    let top_level_names = top_level_names(module);
    let mut enum_variants = enum_variants(module);
    for (enum_name, variants) in external_enum_variants {
        enum_variants
            .entry(enum_name.clone())
            .or_default()
            .extend(variants.iter().cloned());
    }
    for item in &module.items {
        if let Item::Function(function) = item {
            check_function(
                function,
                &functions,
                &top_level_names,
                &enum_variants,
                ambiguous_external_functions,
                &mut diagnostics,
            );
        }
    }

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

fn check_top_level(
    module: &Module,
    local_imports: &HashSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut names = HashSet::new();
    for item in &module.items {
        if let Item::Import {
            path, path_span, ..
        } = item
        {
            check_import(path, *path_span, local_imports, diagnostics);
        }
        let Some((name, span)) = item_name(item) else {
            continue;
        };
        if !names.insert(name.to_string()) {
            diagnostics.push(Diagnostic::new(
                "RESOLVE_DUPLICATE_ITEM",
                format!("duplicate top-level item `{name}`"),
                span,
            ));
        }
    }
}

fn check_import(
    path: &[String],
    span: Span,
    local_imports: &HashSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let import_name = path.join(".");
    if std_api::is_standard_import(path) || local_imports.contains(&import_name) {
        return;
    }

    let supported = std_api::standard_import_names().join("`, `");
    diagnostics.push(Diagnostic::new(
        "RESOLVE_UNKNOWN_IMPORT",
        format!(
            "unknown import `{import_name}`; Stage 1 currently supports standard imports `{supported}` or modules present in the checked module graph"
        ),
        span,
    ));
}

fn check_function(
    function: &Function,
    functions: &HashSet<String>,
    top_level_names: &HashSet<String>,
    enum_variants: &HashMap<String, HashSet<String>>,
    ambiguous_external_functions: &HashSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut locals = HashMap::new();
    for param in &function.params {
        if locals
            .insert(param.name.clone(), Binding { mutable: false })
            .is_some()
        {
            diagnostics.push(Diagnostic::new(
                "RESOLVE_DUPLICATE_LOCAL",
                format!(
                    "duplicate local `{}` in function `{}`",
                    param.name, function.name
                ),
                param.name_span,
            ));
        }
    }
    check_stmts(
        &function.body,
        &mut locals,
        functions,
        top_level_names,
        enum_variants,
        ambiguous_external_functions,
        &function.name,
        diagnostics,
    );
}

fn check_stmts(
    stmts: &[Stmt],
    locals: &mut HashMap<String, Binding>,
    functions: &HashSet<String>,
    top_level_names: &HashSet<String>,
    enum_variants: &HashMap<String, HashSet<String>>,
    ambiguous_external_functions: &HashSet<String>,
    function_name: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for stmt in stmts {
        match stmt {
            Stmt::Let {
                mutable,
                name,
                name_span,
                value,
                ..
            } => {
                check_expr(
                    value,
                    locals,
                    functions,
                    top_level_names,
                    enum_variants,
                    ambiguous_external_functions,
                    function_name,
                    diagnostics,
                );
                if locals
                    .insert(name.clone(), Binding { mutable: *mutable })
                    .is_some()
                {
                    diagnostics.push(Diagnostic::new(
                        "RESOLVE_DUPLICATE_LOCAL",
                        format!("duplicate local `{name}` in function `{function_name}`"),
                        *name_span,
                    ));
                }
            }
            Stmt::Assign {
                name,
                name_span,
                value,
            } => {
                check_expr(
                    value,
                    locals,
                    functions,
                    top_level_names,
                    enum_variants,
                    ambiguous_external_functions,
                    function_name,
                    diagnostics,
                );
                match locals.get(name) {
                    Some(binding) if binding.mutable => {}
                    Some(_) => diagnostics.push(Diagnostic::new(
                        "RESOLVE_ASSIGN_IMMUTABLE",
                        format!(
                            "cannot assign to immutable local `{name}`; declare it with `let mut`"
                        ),
                        *name_span,
                    )),
                    None => diagnostics.push(Diagnostic::new(
                        "RESOLVE_UNKNOWN_NAME",
                        format!("unknown name `{name}` in function `{function_name}`"),
                        *name_span,
                    )),
                }
            }
            Stmt::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                check_expr(
                    condition,
                    locals,
                    functions,
                    top_level_names,
                    enum_variants,
                    ambiguous_external_functions,
                    function_name,
                    diagnostics,
                );
                let mut then_locals = locals.clone();
                check_stmts(
                    then_body,
                    &mut then_locals,
                    functions,
                    top_level_names,
                    enum_variants,
                    ambiguous_external_functions,
                    function_name,
                    diagnostics,
                );
                let mut else_locals = locals.clone();
                check_stmts(
                    else_body,
                    &mut else_locals,
                    functions,
                    top_level_names,
                    enum_variants,
                    ambiguous_external_functions,
                    function_name,
                    diagnostics,
                );
            }
            Stmt::While { condition, body } => {
                check_expr(
                    condition,
                    locals,
                    functions,
                    top_level_names,
                    enum_variants,
                    ambiguous_external_functions,
                    function_name,
                    diagnostics,
                );
                let mut loop_locals = locals.clone();
                check_stmts(
                    body,
                    &mut loop_locals,
                    functions,
                    top_level_names,
                    enum_variants,
                    ambiguous_external_functions,
                    function_name,
                    diagnostics,
                );
            }
            Stmt::Match { value, arms } => {
                check_expr(
                    value,
                    locals,
                    functions,
                    top_level_names,
                    enum_variants,
                    ambiguous_external_functions,
                    function_name,
                    diagnostics,
                );
                for arm in arms {
                    let mut arm_locals = locals.clone();
                    add_pattern_bindings(&arm.pattern, &mut arm_locals);
                    check_stmts(
                        &arm.body,
                        &mut arm_locals,
                        functions,
                        top_level_names,
                        enum_variants,
                        ambiguous_external_functions,
                        function_name,
                        diagnostics,
                    );
                }
            }
            Stmt::Return(Some(value)) | Stmt::Expr(value) => {
                check_expr(
                    value,
                    locals,
                    functions,
                    top_level_names,
                    enum_variants,
                    ambiguous_external_functions,
                    function_name,
                    diagnostics,
                );
            }
            Stmt::Return(None) => {}
        }
    }
}

fn check_expr(
    expr: &Expr,
    locals: &HashMap<String, Binding>,
    functions: &HashSet<String>,
    top_level_names: &HashSet<String>,
    enum_variants: &HashMap<String, HashSet<String>>,
    ambiguous_external_functions: &HashSet<String>,
    function_name: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match expr {
        Expr::Name { name, span } => {
            if !locals.contains_key(name) && !top_level_names.contains(name) {
                diagnostics.push(Diagnostic::new(
                    "RESOLVE_UNKNOWN_NAME",
                    format!("unknown name `{name}` in function `{function_name}`"),
                    *span,
                ));
            }
        }
        Expr::Binary { left, right, .. } => {
            check_expr(
                left,
                locals,
                functions,
                top_level_names,
                enum_variants,
                ambiguous_external_functions,
                function_name,
                diagnostics,
            );
            check_expr(
                right,
                locals,
                functions,
                top_level_names,
                enum_variants,
                ambiguous_external_functions,
                function_name,
                diagnostics,
            );
        }
        Expr::Unary { expr, .. } => {
            check_expr(
                expr,
                locals,
                functions,
                top_level_names,
                enum_variants,
                ambiguous_external_functions,
                function_name,
                diagnostics,
            );
        }
        Expr::Call {
            callee,
            callee_span,
            args,
            ..
        } => {
            if !functions.contains(callee) && !is_enum_variant_call(callee, enum_variants) {
                if ambiguous_external_functions.contains(callee) {
                    diagnostics.push(Diagnostic::new(
                        "RESOLVE_AMBIGUOUS_FUNCTION",
                        format!(
                            "ambiguous imported function `{callee}` in function `{function_name}`; use a qualified call or import alias"
                        ),
                        *callee_span,
                    ));
                } else {
                    diagnostics.push(Diagnostic::new(
                        "RESOLVE_UNKNOWN_FUNCTION",
                        format!("unknown function `{callee}` in function `{function_name}`"),
                        *callee_span,
                    ));
                }
            }
            for arg in args {
                check_expr(
                    arg,
                    locals,
                    functions,
                    top_level_names,
                    enum_variants,
                    ambiguous_external_functions,
                    function_name,
                    diagnostics,
                );
            }
        }
        Expr::StructLiteral { fields, .. } => {
            for field in fields {
                check_expr(
                    &field.value,
                    locals,
                    functions,
                    top_level_names,
                    enum_variants,
                    ambiguous_external_functions,
                    function_name,
                    diagnostics,
                );
            }
        }
        Expr::FieldAccess { base, .. } => {
            check_expr(
                base,
                locals,
                functions,
                top_level_names,
                enum_variants,
                ambiguous_external_functions,
                function_name,
                diagnostics,
            );
        }
        Expr::Int { .. } | Expr::String { .. } | Expr::Bool { .. } => {}
    }
}

#[derive(Debug, Clone, Copy)]
struct Binding {
    mutable: bool,
}

fn add_pattern_bindings(pattern: &Pattern, locals: &mut HashMap<String, Binding>) {
    match pattern {
        Pattern::Name(name) => {
            locals.insert(name.clone(), Binding { mutable: false });
        }
        Pattern::Variant {
            binding: Some(binding),
            ..
        } => {
            locals.insert(binding.clone(), Binding { mutable: false });
        }
        Pattern::Variant { binding: None, .. }
        | Pattern::Int(_)
        | Pattern::String(_)
        | Pattern::Bool(_)
        | Pattern::Wildcard => {}
    }
}

fn is_enum_variant_call(callee: &str, enum_variants: &HashMap<String, HashSet<String>>) -> bool {
    let Some((enum_name, variant)) = callee.rsplit_once('.') else {
        return false;
    };
    enum_variants
        .get(enum_name)
        .is_some_and(|variants| variants.contains(variant))
}

fn function_names(module: &Module) -> HashSet<String> {
    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(function) => Some(function.name.clone()),
            _ => None,
        })
        .collect()
}

fn enum_variants(module: &Module) -> HashMap<String, HashSet<String>> {
    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Enum(decl) => Some((
                decl.name.clone(),
                decl.variants
                    .iter()
                    .map(|variant| variant.name.clone())
                    .collect(),
            )),
            _ => None,
        })
        .collect()
}

fn top_level_names(module: &Module) -> HashSet<String> {
    module
        .items
        .iter()
        .filter_map(|item| item_name(item).map(|(name, _)| name.to_string()))
        .collect()
}

fn item_name(item: &Item) -> Option<(&str, Span)> {
    match item {
        Item::Struct(decl) => Some((&decl.name, decl.name_span)),
        Item::Enum(decl) => Some((&decl.name, decl.name_span)),
        Item::Function(function) => Some((&function.name, function.name_span)),
        Item::ModuleDecl { .. } | Item::Import { .. } => None,
    }
}
