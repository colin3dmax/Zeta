pub mod ast;
#[cfg(feature = "llvm")]
pub mod codegen;
pub mod desugar;
pub mod diagnostic;
pub mod hir;
pub mod lexer;
pub mod line_editor;
pub mod mir;
pub mod module_graph;
pub mod parser;
pub mod repl;
pub mod resolver;
pub mod runtime;
pub mod std_api;
pub mod type_syntax;
pub mod typecheck;
pub mod wasm;

use diagnostic::Diagnostic;

pub fn parse_source(source: &str) -> Result<ast::Module, Vec<Diagnostic>> {
    let tokens = lexer::lex(source)?;
    parser::Parser::new(tokens).parse_module()
}

pub fn dump_ast(source: &str) -> Result<String, Vec<Diagnostic>> {
    let module = parse_source(source)?;
    Ok(module.dump())
}

pub fn dump_hir(source: &str) -> Result<String, Vec<Diagnostic>> {
    let mut module = parse_source(source)?;
    desugar::desugar_try(&mut module)?;
    resolver::resolve(&module)?;
    typecheck::check(&module)?;
    Ok(hir::dump(&module))
}

pub fn dump_mir(source: &str) -> Result<String, Vec<Diagnostic>> {
    let mut module = parse_source(source)?;
    desugar::desugar_try(&mut module)?;
    resolver::resolve(&module)?;
    typecheck::check(&module)?;
    let external_enums = typecheck::standard_external_enums(&module);
    let external_enum_payloads = module_graph::external_enum_payloads(&external_enums);
    let external_enum_type_params = module_graph::external_enum_type_params(&external_enums);
    let program = mir::lower_with_external_enum_variants(
        &module,
        &external_enum_payloads,
        &external_enum_type_params,
    );
    mir::verify(&program)?;
    Ok(mir::dump_program(&program))
}

pub fn check_source(source: &str) -> Result<(), Vec<Diagnostic>> {
    let mut module = parse_source(source)?;
    desugar::desugar_try(&mut module)?;
    resolver::resolve(&module)?;
    typecheck::check(&module)
}

/// Parse → resolve → typecheck → lower → verify a source into a MIR `Program`.
/// The shared front half of `run_source`/`dump_mir`, exposed so the hot-reload
/// runtime (`runtime::HotRuntime`) can lower both the initial program and each
/// hot-swapped revision. Does NOT require a `main` (a service program may export
/// only `init`/`step`).
pub fn lower_source(source: &str) -> Result<mir::Program, Vec<Diagnostic>> {
    let mut module = parse_source(source)?;
    desugar::desugar_try(&mut module)?;
    resolver::resolve(&module)?;
    typecheck::check(&module)?;
    let external_enums = typecheck::standard_external_enums(&module);
    let external_enum_payloads = module_graph::external_enum_payloads(&external_enums);
    let external_enum_type_params = module_graph::external_enum_type_params(&external_enums);
    let program = mir::lower_with_external_enum_variants(
        &module,
        &external_enum_payloads,
        &external_enum_type_params,
    );
    mir::verify(&program)?;
    Ok(program)
}

pub fn run_source(source: &str) -> Result<runtime::Value, Vec<Diagnostic>> {
    let mut module = parse_source(source)?;
    desugar::desugar_try(&mut module)?;
    resolver::resolve(&module)?;
    typecheck::check(&module)?;
    let external_enums = typecheck::standard_external_enums(&module);
    let external_enum_payloads = module_graph::external_enum_payloads(&external_enums);
    let external_enum_type_params = module_graph::external_enum_type_params(&external_enums);
    let program = mir::lower_with_external_enum_variants(
        &module,
        &external_enum_payloads,
        &external_enum_type_params,
    );
    mir::verify(&program)?;
    runtime::run_mir(&program)
}

pub fn run_repl_source(source: &str) -> Result<runtime::Value, Vec<Diagnostic>> {
    let mut module = parse_source(source)?;
    desugar::desugar_try(&mut module)?;
    resolver::resolve(&module)?;
    runtime::run(&module)
}

pub fn eval_repl_source(
    session: &mut runtime::ReplSession,
    source: &str,
) -> Result<runtime::Value, Vec<Diagnostic>> {
    let mut module = parse_source(source)?;
    desugar::desugar_try(&mut module)?;
    session.eval_module(&module)
}

pub fn repl_source_for_line(line: &str) -> String {
    let trimmed = line.trim();
    if starts_with_top_level_item(trimmed) {
        trimmed.to_string()
    } else {
        format!("fn repl() {{\n  {trimmed}\n}}")
    }
}

pub fn repl_run_source_for_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() || starts_with_top_level_item(trimmed) {
        return None;
    }
    if trimmed.ends_with(';') {
        Some(format!("fn main() {{\n  {trimmed}\n}}"))
    } else {
        Some(format!("fn main() {{\n  return {trimmed};\n}}"))
    }
}

fn starts_with_top_level_item(line: &str) -> bool {
    matches!(
        line.split_whitespace().next(),
        Some("module" | "import" | "as" | "export" | "fn" | "struct" | "enum")
    )
}
