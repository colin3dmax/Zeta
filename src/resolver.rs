use crate::ast::{Expr, Function, Item, LambdaBody, Module, Pattern, Stmt};
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
    functions.extend(standard_function_names(module));
    // Trait method names are callable via UFCS (dispatched per-backend to an
    // `impl`'s `{method}${TargetBase}`), so accept them as known call targets.
    functions.extend(module.trait_method_names());
    // Generic array intrinsics (`array_push`/`array_repeat`) are always callable.
    functions.insert("array_push".to_string());
    functions.insert("array_repeat".to_string());
    // Raw-pointer intrinsics (unsafe, native-only) are always callable too.
    for name in [
        "ptr_from_addr",
        "ptr_addr",
        "ptr_read",
        "ptr_write",
        "ptr_offset",
        "array_data_addr",
    ] {
        functions.insert(name.to_string());
    }
    let mut top_level_names = top_level_names(module);
    let mut enum_variants = enum_variants(module);
    for standard_enum in standard_enum_variants(module) {
        top_level_names.insert(standard_enum.0.clone());
        enum_variants
            .entry(standard_enum.0)
            .or_default()
            .extend(standard_enum.1);
    }
    for (enum_name, variants) in external_enum_variants {
        top_level_names.insert(enum_name.clone());
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
    let standard_top_level_names = standard_top_level_names(module);
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
        if standard_top_level_names.contains(name) {
            diagnostics.push(Diagnostic::new(
                "RESOLVE_DUPLICATE_ITEM",
                format!("top-level item `{name}` conflicts with an imported std.core item"),
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
            Stmt::Assign { target, value } => {
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
                match target {
                    Expr::Name { name, span } => match locals.get(name) {
                        Some(binding) if binding.mutable => {}
                        Some(_) => diagnostics.push(Diagnostic::new(
                            "RESOLVE_ASSIGN_IMMUTABLE",
                            format!(
                                "cannot assign to immutable local `{name}`; declare it with `let mut`"
                            ),
                            *span,
                        )),
                        None => diagnostics.push(Diagnostic::new(
                            "RESOLVE_UNKNOWN_NAME",
                            format!("unknown name `{name}` in function `{function_name}`"),
                            *span,
                        )),
                    },
                    Expr::FieldAccess { .. } | Expr::Index { .. } => {
                        check_expr(
                            target,
                            locals,
                            functions,
                            top_level_names,
                            enum_variants,
                            ambiguous_external_functions,
                            function_name,
                            diagnostics,
                        );
                        if let Some((root, root_span)) = target.assign_root() {
                            match locals.get(root) {
                                Some(binding) if binding.mutable => {}
                                Some(_) => diagnostics.push(Diagnostic::new(
                                    "RESOLVE_ASSIGN_IMMUTABLE",
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
                        "RESOLVE_ASSIGN_TARGET",
                        "invalid assignment target",
                        target.span(),
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
            Stmt::ForIn {
                binding,
                iterable,
                body,
                ..
            } => {
                check_expr(
                    iterable,
                    locals,
                    functions,
                    top_level_names,
                    enum_variants,
                    ambiguous_external_functions,
                    function_name,
                    diagnostics,
                );
                let mut loop_locals = locals.clone();
                loop_locals.insert(binding.clone(), Binding { mutable: false });
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
            Stmt::ForC {
                init,
                condition,
                step,
                body,
            } => {
                let mut loop_locals = locals.clone();
                // init (often `let mut i = 0`) declares its binding in loop_locals.
                check_stmts(
                    std::slice::from_ref(init.as_ref()),
                    &mut loop_locals,
                    functions,
                    top_level_names,
                    enum_variants,
                    ambiguous_external_functions,
                    function_name,
                    diagnostics,
                );
                check_expr(
                    condition,
                    &loop_locals,
                    functions,
                    top_level_names,
                    enum_variants,
                    ambiguous_external_functions,
                    function_name,
                    diagnostics,
                );
                check_stmts(
                    std::slice::from_ref(step.as_ref()),
                    &mut loop_locals,
                    functions,
                    top_level_names,
                    enum_variants,
                    ambiguous_external_functions,
                    function_name,
                    diagnostics,
                );
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
            Stmt::Return(None) | Stmt::Break { .. } | Stmt::Continue { .. } => {}
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
        Expr::Try { .. } => unreachable!("`?` is desugared before resolve"),
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
            // A callee that names a local binding is an indirect call through a
            // closure value, which is resolved like any other name reference.
            if !functions.contains(callee)
                && !is_enum_variant_call(callee, enum_variants)
                && !locals.contains_key(callee)
            {
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
        Expr::ArrayLiteral { elements, .. } | Expr::Tuple { elements, .. } => {
            for element in elements {
                check_expr(
                    element,
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
        Expr::Lambda { params, body, .. } => {
            // The body sees the enclosing scope plus the lambda's own params
            // (captures resolve against the outer locals, which stay visible).
            let mut body_locals = locals.clone();
            for param in params {
                body_locals.insert(param.name.clone(), Binding { mutable: false });
            }
            match body {
                LambdaBody::Expr(e) => check_expr(
                    e,
                    &mut body_locals,
                    functions,
                    top_level_names,
                    enum_variants,
                    ambiguous_external_functions,
                    function_name,
                    diagnostics,
                ),
                LambdaBody::Block(stmts) => check_stmts(
                    stmts,
                    &mut body_locals,
                    functions,
                    top_level_names,
                    enum_variants,
                    ambiguous_external_functions,
                    function_name,
                    diagnostics,
                ),
            }
        }
        Expr::Index { base, index, .. } => {
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
            check_expr(
                index,
                locals,
                functions,
                top_level_names,
                enum_variants,
                ambiguous_external_functions,
                function_name,
                diagnostics,
            );
        }
        Expr::Range { start, end, .. } => {
            check_expr(
                start,
                locals,
                functions,
                top_level_names,
                enum_variants,
                ambiguous_external_functions,
                function_name,
                diagnostics,
            );
            check_expr(
                end,
                locals,
                functions,
                top_level_names,
                enum_variants,
                ambiguous_external_functions,
                function_name,
                diagnostics,
            );
        }
        Expr::Int { .. } | Expr::Float { .. } | Expr::String { .. } | Expr::Bool { .. } => {}
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

fn standard_enum_variants(module: &Module) -> Vec<(String, HashSet<String>)> {
    let imports_std_core = module.items.iter().any(|item| match item {
        Item::Import { path, .. } => std_api::is_std_core_import(path),
        _ => false,
    });
    let imports_std_io = module.items.iter().any(|item| match item {
        Item::Import { path, .. } => std_api::is_std_io_import(path),
        _ => false,
    });

    let mut variants = Vec::new();
    if imports_std_core {
        variants.extend(standard_enum_variant_names(std_api::core_enums()));
    }
    if imports_std_io {
        variants.extend(standard_enum_variant_names(std_api::io_enums()));
    }
    variants
}

fn standard_top_level_names(module: &Module) -> HashSet<String> {
    let imports_std_core = module.items.iter().any(|item| match item {
        Item::Import { path, .. } => std_api::is_std_core_import(path),
        _ => false,
    });
    let imports_std_io = module.items.iter().any(|item| match item {
        Item::Import { path, .. } => std_api::is_std_io_import(path),
        _ => false,
    });

    let mut names = HashSet::new();
    if imports_std_core {
        names.extend(standard_enum_names(std_api::core_enums()));
    }
    if imports_std_io {
        names.extend(standard_enum_names(std_api::io_enums()));
    }
    names
}

fn standard_function_names(module: &Module) -> HashSet<String> {
    let imports_std_core = module.items.iter().any(|item| match item {
        Item::Import { path, .. } => std_api::is_std_core_import(path),
        _ => false,
    });
    let imports_std_io = module.items.iter().any(|item| match item {
        Item::Import { path, .. } => std_api::is_std_io_import(path),
        _ => false,
    });

    let mut names = HashSet::new();
    if imports_std_core {
        names.extend(standard_function_name_set(std_api::core_functions()));
    }
    if imports_std_io {
        names.extend(standard_function_name_set(std_api::io_functions()));
    }
    names
}

fn standard_enum_variant_names(enums: &[std_api::StandardEnum]) -> Vec<(String, HashSet<String>)> {
    enums
        .iter()
        .map(|standard_enum| {
            (
                standard_enum.name.to_string(),
                standard_enum
                    .variants
                    .iter()
                    .map(|variant| variant.name.to_string())
                    .collect(),
            )
        })
        .collect()
}

fn standard_enum_names(enums: &[std_api::StandardEnum]) -> HashSet<String> {
    enums
        .iter()
        .map(|standard_enum| standard_enum.name.to_string())
        .collect()
}

fn standard_function_name_set(functions: &[std_api::StandardFunction]) -> HashSet<String> {
    functions
        .iter()
        .map(|function| function.name.to_string())
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
        Item::Trait(decl) => Some((&decl.name, decl.name_span)),
        // `impl` blocks contribute no top-level name themselves.
        Item::Impl(_) => None,
        Item::ModuleDecl { .. } | Item::Import { .. } => None,
    }
}
