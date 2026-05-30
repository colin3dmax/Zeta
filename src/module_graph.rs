use crate::ast::{Function, Item, Module};
use crate::diagnostic::{Diagnostic, Span};
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

    let module_infos = module_infos(&modules, &mut errors);
    let local_imports = module_infos.keys().cloned().collect::<HashSet<_>>();
    for parsed in &modules {
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
    for import in local_import_paths(module) {
        let Some(info) = module_infos.get(&import) else {
            continue;
        };
        for function in &info.exported_functions {
            if seen.insert(function.name.clone()) {
                functions.push(function.clone());
            }
        }
    }
    functions
}

fn local_import_paths(module: &Module) -> Vec<String> {
    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Import { path, .. } => Some(path.join(".")),
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
