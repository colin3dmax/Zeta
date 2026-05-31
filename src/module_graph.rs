use crate::ast::{Function, Item, Module};
use crate::diagnostic::{Diagnostic, Span};
use crate::mir::{self, MirExpr, MirStmt, Program};
use crate::runtime::{self, Value};
use crate::typecheck::ExternalFunction;
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
        let external_function_names = external_functions
            .iter()
            .map(|function| function.name.clone())
            .collect::<HashSet<_>>();
        collect_result(
            &parsed.path,
            &parsed.source,
            resolver::resolve_with_imports_and_functions(
                &parsed.module,
                &local_imports,
                &external_function_names,
            ),
            &mut errors,
        );
        collect_result(
            &parsed.path,
            &parsed.source,
            typecheck::check_with_external_functions(&parsed.module, &external_functions),
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
        MirExpr::Load(_)
        | MirExpr::Int(_)
        | MirExpr::String(_)
        | MirExpr::Bool(_)
        | MirExpr::EnumVariant { .. } => {}
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
                    exported_functions: exported_functions(&parsed.module),
                },
            );
        }
    }
    infos
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
    for import in local_imports(module) {
        let Some(info) = module_infos.get(&import.path) else {
            continue;
        };
        for function in &info.exported_functions {
            if seen.insert(function.name.clone()) {
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
                };
                if seen.insert(alias_qualified.name.clone()) {
                    functions.push(alias_qualified);
                }
            }
        }
    }
    functions
}

fn imported_call_targets(
    module: &Module,
    module_infos: &HashMap<String, ModuleInfo>,
) -> HashMap<String, String> {
    let mut targets = HashMap::new();
    for import in local_imports(module) {
        let Some(info) = module_infos.get(&import.path) else {
            continue;
        };
        for function in &info.exported_functions {
            targets
                .entry(function.name.clone())
                .or_insert_with(|| format!("{}.{}", import.path, function.name));
            if let Some(alias) = &import.alias {
                targets
                    .entry(format!("{alias}.{}", function.name))
                    .or_insert_with(|| format!("{}.{}", import.path, function.name));
            }
        }
    }
    targets
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

fn exported_functions(module: &Module) -> Vec<ExternalFunction> {
    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(function) if function.exported => Some(external_function(function)),
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

fn external_function(function: &Function) -> ExternalFunction {
    ExternalFunction {
        name: function.name.clone(),
        params: function
            .params
            .iter()
            .map(|param| param.ty.clone())
            .collect(),
        return_type: function.return_type.clone(),
    }
}

fn qualified_function(function: &ExternalFunction, module_name: &str) -> ExternalFunction {
    ExternalFunction {
        name: format!("{module_name}.{}", function.name),
        params: function.params.clone(),
        return_type: function.return_type.clone(),
    }
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

struct ModuleInfo {
    exported_functions: Vec<ExternalFunction>,
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
