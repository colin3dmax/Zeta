use crate::ast::{Item, Module};
use crate::diagnostic::{Diagnostic, Span};
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

    let local_imports = local_module_names(&modules, &mut errors);
    for parsed in &modules {
        collect_result(
            &parsed.path,
            &parsed.source,
            resolver::resolve_with_imports(&parsed.module, &local_imports),
            &mut errors,
        );
        collect_result(
            &parsed.path,
            &parsed.source,
            typecheck::check(&parsed.module),
            &mut errors,
        );
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn local_module_names(
    modules: &[ParsedSource],
    errors: &mut Vec<SourceDiagnostics>,
) -> HashSet<String> {
    let mut seen: HashMap<String, (&str, Span)> = HashMap::new();
    let mut names = HashSet::new();
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
            names.insert(name.to_string());
        }
    }
    names
}

fn module_decl(module: &Module) -> Option<(&str, Span)> {
    module.items.iter().find_map(|item| match item {
        Item::ModuleDecl { name, name_span } => Some((name.as_str(), *name_span)),
        _ => None,
    })
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
