pub mod ast;
pub mod diagnostic;
pub mod hir;
pub mod lexer;
pub mod line_editor;
pub mod mir;
pub mod parser;
pub mod repl;
pub mod resolver;
pub mod runtime;
pub mod std_api;
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
    let module = parse_source(source)?;
    resolver::resolve(&module)?;
    typecheck::check(&module)?;
    Ok(hir::dump(&module))
}

pub fn dump_mir(source: &str) -> Result<String, Vec<Diagnostic>> {
    let module = parse_source(source)?;
    resolver::resolve(&module)?;
    typecheck::check(&module)?;
    Ok(mir::dump(&module))
}

pub fn check_source(source: &str) -> Result<(), Vec<Diagnostic>> {
    let module = parse_source(source)?;
    resolver::resolve(&module)?;
    typecheck::check(&module)
}

pub fn run_source(source: &str) -> Result<runtime::Value, Vec<Diagnostic>> {
    let module = parse_source(source)?;
    resolver::resolve(&module)?;
    typecheck::check(&module)?;
    runtime::run(&module)
}

pub fn run_repl_source(source: &str) -> Result<runtime::Value, Vec<Diagnostic>> {
    let module = parse_source(source)?;
    resolver::resolve(&module)?;
    runtime::run(&module)
}

pub fn eval_repl_source(
    session: &mut runtime::ReplSession,
    source: &str,
) -> Result<runtime::Value, Vec<Diagnostic>> {
    let module = parse_source(source)?;
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
        Some("module" | "import" | "export" | "fn" | "struct" | "enum")
    )
}
