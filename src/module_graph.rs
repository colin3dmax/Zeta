use crate::ast::{EnumDecl, Function, Item, Module, StructDecl};
use crate::diagnostic::{Diagnostic, Span};
use crate::mir::{self, MirExpr, MirStmt, Program};
use crate::runtime::{self, Value};
use crate::typecheck::{ExternalEnum, ExternalFunction, ExternalStruct};
use crate::{parser, resolver, typecheck};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct SourceFile {
    pub path: String,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct SourceDiagnostics {
    pub path: String,
    pub source: String,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn check_sources(files: &[SourceFile]) -> Result<(), Vec<SourceDiagnostics>> {
    let modules = parse_sources(files)?;
    check_parsed_sources(&modules)
}

pub fn run_sources(files: &[SourceFile]) -> Result<Value, Vec<SourceDiagnostics>> {
    let modules = parse_sources(files)?;
    check_parsed_sources(&modules)?;
    let main_modules = modules
        .iter()
        .filter(|parsed| has_function(&parsed.module, "main"))
        .collect::<Vec<_>>();
    if main_modules.is_empty() {
        return Err(vec![source_error(
            modules.first(),
            Diagnostic::new(
                "RUNTIME_NO_MAIN",
                "expected a `main` function",
                Span::new(0, 0),
            ),
        )]);
    }
    if main_modules.len() > 1 {
        return Err(main_modules
            .iter()
            .map(|parsed| {
                source_error(
                    Some(parsed),
                    Diagnostic::new(
                        "RUNTIME_DUPLICATE_MAIN",
                        "directory run requires exactly one `main` function",
                        Span::new(0, 0),
                    ),
                )
            })
            .collect());
    }

    let module_infos = module_infos(&modules, &mut Vec::new());
    let program = combined_program(&modules, main_modules[0].path.as_str(), &module_infos);
    runtime::run_mir(&program).map_err(|diagnostics| {
        let main = main_modules[0];
        vec![SourceDiagnostics {
            path: main.path.clone(),
            source: main.source.clone(),
            diagnostics,
        }]
    })
}

fn parse_sources(files: &[SourceFile]) -> Result<Vec<ParsedSource>, Vec<SourceDiagnostics>> {
    let mut modules = Vec::new();
    let mut errors = Vec::new();

    for file in files {
        match crate::lexer::lex(&file.source)
            .and_then(|tokens| parser::Parser::new(tokens).parse_module())
        {
            Ok(module) => modules.push(ParsedSource {
                path: file.path.clone(),
                source: file.source.clone(),
                module,
            }),
            Err(diagnostics) => errors.push(SourceDiagnostics {
                path: file.path.clone(),
                source: file.source.clone(),
                diagnostics,
            }),
        }
    }

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(modules)
}

fn check_parsed_sources(modules: &[ParsedSource]) -> Result<(), Vec<SourceDiagnostics>> {
    let mut errors = Vec::new();
    let module_infos = module_infos(&modules, &mut errors);
    let local_imports = module_infos.keys().cloned().collect::<HashSet<_>>();
    for parsed in modules {
        let external_functions = imported_external_functions(&parsed.module, &module_infos);
        let external_structs = imported_external_structs(&parsed.module, &module_infos);
        let external_enums = imported_external_enums(&parsed.module, &module_infos);
        let ambiguous_functions = ambiguous_external_function_names(&parsed.module, &module_infos);
        let external_function_names = external_functions
            .iter()
            .map(|function| function.name.clone())
            .collect::<HashSet<_>>();
        collect_result(
            &parsed.path,
            &parsed.source,
            resolver::resolve_with_imports_functions_and_ambiguous(
                &parsed.module,
                &local_imports,
                &external_function_names,
                &ambiguous_functions,
            ),
            &mut errors,
        );
        collect_result(
            &parsed.path,
            &parsed.source,
            typecheck::check_with_external_items(
                &parsed.module,
                &external_functions,
                &external_structs,
                &external_enums,
            ),
            &mut errors,
        );
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn combined_program(
    modules: &[ParsedSource],
    main_path: &str,
    module_infos: &HashMap<String, ModuleInfo>,
) -> Program {
    let mut program = Program {
        enums: Vec::new(),
        functions: Vec::new(),
    };
    for parsed in modules {
        let current_module = module_decl(&parsed.module).map(|(name, _)| name.to_string());
        let imported_targets = imported_call_targets(&parsed.module, module_infos);
        let is_main_module = parsed.path == main_path;
        let mut lowered = mir::lower(&parsed.module);
        rewrite_program_calls(
            &mut lowered,
            current_module.as_deref(),
            &imported_targets,
            is_main_module,
        );
        program.enums.extend(lowered.enums);
        program.functions.extend(lowered.functions);
    }
    program
}

fn rewrite_program_calls(
    program: &mut Program,
    current_module: Option<&str>,
    imported_targets: &HashMap<String, String>,
    is_main_module: bool,
) {
    let local_functions = program
        .functions
        .iter()
        .map(|function| function.name.clone())
        .collect::<HashSet<_>>();
    for function in &mut program.functions {
        let original_name = function.name.clone();
        rewrite_stmts(
            &mut function.body,
            current_module,
            imported_targets,
            &local_functions,
            is_main_module,
        );
        if original_name != "main" || !is_main_module {
            if let Some(module_name) = current_module {
                function.name = format!("{module_name}.{original_name}");
            }
        }
    }
}

fn rewrite_stmts(
    stmts: &mut [MirStmt],
    current_module: Option<&str>,
    imported_targets: &HashMap<String, String>,
    local_functions: &HashSet<String>,
    is_main_module: bool,
) {
    for stmt in stmts {
        match stmt {
            MirStmt::Local { value, .. }
            | MirStmt::Store { value, .. }
            | MirStmt::Return(Some(value))
            | MirStmt::Drop(value) => rewrite_expr(
                value,
                current_module,
                imported_targets,
                local_functions,
                is_main_module,
            ),
            MirStmt::If {
                condition,
                then_body,
                else_body,
            } => {
                rewrite_expr(
                    condition,
                    current_module,
                    imported_targets,
                    local_functions,
                    is_main_module,
                );
                rewrite_stmts(
                    then_body,
                    current_module,
                    imported_targets,
                    local_functions,
                    is_main_module,
                );
                rewrite_stmts(
                    else_body,
                    current_module,
                    imported_targets,
                    local_functions,
                    is_main_module,
                );
            }
            MirStmt::While { condition, body } => {
                rewrite_expr(
                    condition,
                    current_module,
                    imported_targets,
                    local_functions,
                    is_main_module,
                );
                rewrite_stmts(
                    body,
                    current_module,
                    imported_targets,
                    local_functions,
                    is_main_module,
                );
            }
            MirStmt::Match { value, arms } => {
                rewrite_expr(
                    value,
                    current_module,
                    imported_targets,
                    local_functions,
                    is_main_module,
                );
                for arm in arms {
                    rewrite_stmts(
                        &mut arm.body,
                        current_module,
                        imported_targets,
                        local_functions,
                        is_main_module,
                    );
                }
            }
            MirStmt::Return(None) => {}
        }
    }
}

fn rewrite_expr(
    expr: &mut MirExpr,
    current_module: Option<&str>,
    imported_targets: &HashMap<String, String>,
    local_functions: &HashSet<String>,
    is_main_module: bool,
) {
    match expr {
        MirExpr::Binary { left, right, .. } => {
            rewrite_expr(
                left,
                current_module,
                imported_targets,
                local_functions,
                is_main_module,
            );
            rewrite_expr(
                right,
                current_module,
                imported_targets,
                local_functions,
                is_main_module,
            );
        }
        MirExpr::Unary { expr, .. } | MirExpr::FieldAccess { base: expr, .. } => rewrite_expr(
            expr,
            current_module,
            imported_targets,
            local_functions,
            is_main_module,
        ),
        MirExpr::Call { callee, args } => {
            for arg in args {
                rewrite_expr(
                    arg,
                    current_module,
                    imported_targets,
                    local_functions,
                    is_main_module,
                );
            }
            if let Some(target) = imported_targets.get(callee) {
                *callee = target.clone();
            } else if callee.contains('.') {
                return;
            } else if !is_main_module && local_functions.contains(callee) {
                if let Some(module_name) = current_module {
                    *callee = format!("{module_name}.{callee}");
                }
            }
        }
        MirExpr::StructLiteral { fields, .. } => {
            for field in fields {
                rewrite_expr(
                    &mut field.value,
                    current_module,
                    imported_targets,
                    local_functions,
                    is_main_module,
                );
            }
        }
        MirExpr::EnumVariant { payload, .. } => {
            if let Some(payload) = payload {
                rewrite_expr(
                    payload,
                    current_module,
                    imported_targets,
                    local_functions,
                    is_main_module,
                );
            }
        }
        MirExpr::Load(_) | MirExpr::Int(_) | MirExpr::String(_) | MirExpr::Bool(_) => {}
    }
}

fn module_infos(
    modules: &[ParsedSource],
    errors: &mut Vec<SourceDiagnostics>,
) -> HashMap<String, ModuleInfo> {
    let mut seen: HashMap<String, (&str, Span)> = HashMap::new();
    let mut infos = HashMap::new();
    for parsed in modules {
        let Some((name, span)) = module_decl(&parsed.module) else {
            continue;
        };
        if let Some((first_path, _)) = seen.get(name) {
            errors.push(SourceDiagnostics {
                path: parsed.path.clone(),
                source: parsed.source.clone(),
                diagnostics: vec![Diagnostic::new(
                    "RESOLVE_DUPLICATE_MODULE",
                    format!("duplicate module `{name}`; first declared in {first_path}"),
                    span,
                )],
            });
        } else {
            seen.insert(name.to_string(), (&parsed.path, span));
            infos.insert(
                name.to_string(),
                ModuleInfo {
                    exported_functions: exported_functions(&parsed.module, name),
                    exported_structs: exported_structs(&parsed.module),
                    exported_enums: exported_enums(&parsed.module),
                    reexport_imports: exported_imports(&parsed.module),
                },
            );
        }
    }
    expand_reexports(&mut infos, modules, errors);
    infos
}

fn expand_reexports(
    infos: &mut HashMap<String, ModuleInfo>,
    modules: &[ParsedSource],
    errors: &mut Vec<SourceDiagnostics>,
) {
    for _ in 0..infos.len() {
        let snapshot = infos.clone();
        let mut changed = false;
        for info in infos.values_mut() {
            let mut exported = info.exported_functions.clone();
            let mut structs = info.exported_structs.clone();
            let mut enums = info.exported_enums.clone();
            for import in &info.reexport_imports {
                if let Some(imported) = snapshot.get(&import.path) {
                    exported.extend(imported.exported_functions.clone());
                    structs.extend(imported.exported_structs.clone());
                    enums.extend(imported.exported_enums.clone());
                }
            }
            changed |=
                replace_exported_functions_if_changed(&mut info.exported_functions, exported);
            changed |= replace_exported_structs_if_changed(&mut info.exported_structs, structs);
            changed |= replace_exported_enums_if_changed(&mut info.exported_enums, enums);
        }
        if !changed {
            break;
        }
    }

    for parsed in modules {
        let Some((name, span)) = module_decl(&parsed.module) else {
            continue;
        };
        let Some(info) = infos.get(name) else {
            continue;
        };
        if let Some(duplicate) = duplicate_export_name(&info.exported_functions) {
            errors.push(SourceDiagnostics {
                path: parsed.path.clone(),
                source: parsed.source.clone(),
                diagnostics: vec![Diagnostic::new(
                    "RESOLVE_AMBIGUOUS_REEXPORT",
                    format!("module `{name}` exports multiple functions named `{duplicate}`"),
                    span,
                )],
            });
        }
    }
}

fn replace_exported_functions_if_changed(
    current: &mut Vec<ExternalFunction>,
    mut next: Vec<ExternalFunction>,
) -> bool {
    next.sort_by(|left, right| left.name.cmp(&right.name));
    next.dedup_by(|left, right| {
        left.name == right.name
            && left.params == right.params
            && left.return_type == right.return_type
            && left.target_name == right.target_name
    });
    if *current == next {
        return false;
    }
    *current = next;
    true
}

fn replace_exported_structs_if_changed(
    current: &mut Vec<ExternalStruct>,
    mut next: Vec<ExternalStruct>,
) -> bool {
    next.sort_by(|left, right| left.name.cmp(&right.name));
    next.dedup();
    if *current == next {
        return false;
    }
    *current = next;
    true
}

fn replace_exported_enums_if_changed(
    current: &mut Vec<ExternalEnum>,
    mut next: Vec<ExternalEnum>,
) -> bool {
    next.sort_by(|left, right| left.name.cmp(&right.name));
    next.dedup();
    if *current == next {
        return false;
    }
    *current = next;
    true
}

fn duplicate_export_name(functions: &[ExternalFunction]) -> Option<String> {
    let mut seen = HashSet::new();
    for function in functions {
        if !seen.insert(function.name.as_str()) {
            return Some(function.name.clone());
        }
    }
    None
}

fn module_decl(module: &Module) -> Option<(&str, Span)> {
    module.items.iter().find_map(|item| match item {
        Item::ModuleDecl { name, name_span } => Some((name.as_str(), *name_span)),
        _ => None,
    })
}

fn imported_external_functions(
    module: &Module,
    module_infos: &HashMap<String, ModuleInfo>,
) -> Vec<ExternalFunction> {
    let mut functions = Vec::new();
    let mut seen = HashSet::new();
    let ambiguous_short_names = ambiguous_external_function_names(module, module_infos);
    for import in local_imports(module) {
        let Some(info) = module_infos.get(&import.path) else {
            continue;
        };
        for function in &info.exported_functions {
            if !ambiguous_short_names.contains(&function.name) && seen.insert(function.name.clone())
            {
                functions.push(function.clone());
            }
            let qualified = qualified_function(function, &import.path);
            if seen.insert(qualified.name.clone()) {
                functions.push(qualified);
            }
            if let Some(alias) = &import.alias {
                let alias_qualified = ExternalFunction {
                    name: format!("{alias}.{}", function.name),
                    params: function.params.clone(),
                    return_type: function.return_type.clone(),
                    target_name: function.target_name.clone(),
                };
                if seen.insert(alias_qualified.name.clone()) {
                    functions.push(alias_qualified);
                }
            }
        }
    }
    functions
}

fn imported_external_structs(
    module: &Module,
    module_infos: &HashMap<String, ModuleInfo>,
) -> Vec<ExternalStruct> {
    let mut structs = Vec::new();
    let mut seen = HashSet::new();
    for import in local_imports(module) {
        let Some(info) = module_infos.get(&import.path) else {
            continue;
        };
        for external_struct in &info.exported_structs {
            if seen.insert(external_struct.name.clone()) {
                structs.push(external_struct.clone());
            }
        }
    }
    structs
}

fn imported_external_enums(
    module: &Module,
    module_infos: &HashMap<String, ModuleInfo>,
) -> Vec<ExternalEnum> {
    let mut enums = Vec::new();
    let mut seen = HashSet::new();
    for import in local_imports(module) {
        let Some(info) = module_infos.get(&import.path) else {
            continue;
        };
        for external_enum in &info.exported_enums {
            if seen.insert(external_enum.name.clone()) {
                enums.push(external_enum.clone());
            }
        }
    }
    enums
}

fn imported_call_targets(
    module: &Module,
    module_infos: &HashMap<String, ModuleInfo>,
) -> HashMap<String, String> {
    let mut targets = HashMap::new();
    let ambiguous_short_names = ambiguous_external_function_names(module, module_infos);
    for import in local_imports(module) {
        let Some(info) = module_infos.get(&import.path) else {
            continue;
        };
        for function in &info.exported_functions {
            if !ambiguous_short_names.contains(&function.name) {
                targets
                    .entry(function.name.clone())
                    .or_insert_with(|| function_target(function, &import.path));
            }
            targets
                .entry(format!("{}.{}", import.path, function.name))
                .or_insert_with(|| function_target(function, &import.path));
            if let Some(alias) = &import.alias {
                targets
                    .entry(format!("{alias}.{}", function.name))
                    .or_insert_with(|| function_target(function, &import.path));
            }
        }
    }
    targets
}

fn ambiguous_external_function_names(
    module: &Module,
    module_infos: &HashMap<String, ModuleInfo>,
) -> HashSet<String> {
    let mut origins: HashMap<String, HashSet<String>> = HashMap::new();
    for import in local_imports(module) {
        let Some(info) = module_infos.get(&import.path) else {
            continue;
        };
        for function in &info.exported_functions {
            origins
                .entry(function.name.clone())
                .or_default()
                .insert(import.path.clone());
        }
    }
    origins
        .into_iter()
        .filter_map(|(name, origins)| (origins.len() > 1).then_some(name))
        .collect()
}

#[derive(Debug, Clone)]
struct LocalImport {
    path: String,
    alias: Option<String>,
}

fn local_imports(module: &Module) -> Vec<LocalImport> {
    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Import { path, alias, .. } => Some(LocalImport {
                path: path.join("."),
                alias: alias.clone(),
            }),
            _ => None,
        })
        .collect()
}

fn exported_imports(module: &Module) -> Vec<LocalImport> {
    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Import {
                exported: true,
                path,
                alias,
                ..
            } => Some(LocalImport {
                path: path.join("."),
                alias: alias.clone(),
            }),
            _ => None,
        })
        .collect()
}

fn exported_functions(module: &Module, module_name: &str) -> Vec<ExternalFunction> {
    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(function) if function.exported => {
                Some(external_function(function, module_name))
            }
            _ => None,
        })
        .collect()
}

fn exported_structs(module: &Module) -> Vec<ExternalStruct> {
    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(decl) if decl.exported => Some(external_struct(decl)),
            _ => None,
        })
        .collect()
}

fn exported_enums(module: &Module) -> Vec<ExternalEnum> {
    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Enum(decl) if decl.exported => Some(external_enum(decl)),
            _ => None,
        })
        .collect()
}

fn has_function(module: &Module, name: &str) -> bool {
    module.items.iter().any(|item| match item {
        Item::Function(function) => function.name == name,
        _ => false,
    })
}

fn external_function(function: &Function, module_name: &str) -> ExternalFunction {
    ExternalFunction {
        name: function.name.clone(),
        params: function
            .params
            .iter()
            .map(|param| param.ty.clone())
            .collect(),
        return_type: function.return_type.clone(),
        target_name: Some(format!("{module_name}.{}", function.name)),
    }
}

fn external_struct(decl: &StructDecl) -> ExternalStruct {
    ExternalStruct {
        name: decl.name.clone(),
        fields: decl
            .fields
            .iter()
            .map(|field| (field.name.clone(), field.ty.clone()))
            .collect(),
    }
}

fn external_enum(decl: &EnumDecl) -> ExternalEnum {
    ExternalEnum {
        name: decl.name.clone(),
        variants: decl
            .variants
            .iter()
            .map(|variant| (variant.name.clone(), variant.payload_type.clone()))
            .collect(),
    }
}

fn qualified_function(function: &ExternalFunction, module_name: &str) -> ExternalFunction {
    ExternalFunction {
        name: format!("{module_name}.{}", function.name),
        params: function.params.clone(),
        return_type: function.return_type.clone(),
        target_name: function.target_name.clone(),
    }
}

fn function_target(function: &ExternalFunction, imported_module: &str) -> String {
    function
        .target_name
        .clone()
        .unwrap_or_else(|| format!("{imported_module}.{}", function.name))
}

fn collect_result(
    path: &str,
    source: &str,
    result: Result<(), Vec<Diagnostic>>,
    errors: &mut Vec<SourceDiagnostics>,
) {
    if let Err(diagnostics) = result {
        errors.push(SourceDiagnostics {
            path: path.to_string(),
            source: source.to_string(),
            diagnostics,
        });
    }
}

struct ParsedSource {
    path: String,
    source: String,
    module: Module,
}

#[derive(Clone)]
struct ModuleInfo {
    exported_functions: Vec<ExternalFunction>,
    exported_structs: Vec<ExternalStruct>,
    exported_enums: Vec<ExternalEnum>,
    reexport_imports: Vec<LocalImport>,
}

fn source_error(parsed: Option<&ParsedSource>, diagnostic: Diagnostic) -> SourceDiagnostics {
    match parsed {
        Some(parsed) => SourceDiagnostics {
            path: parsed.path.clone(),
            source: parsed.source.clone(),
            diagnostics: vec![diagnostic],
        },
        None => SourceDiagnostics {
            path: "<module graph>".to_string(),
            source: String::new(),
            diagnostics: vec![diagnostic],
        },
    }
}
