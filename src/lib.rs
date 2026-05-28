pub mod ast;
pub mod diagnostic;
pub mod lexer;
pub mod resolver;
pub mod parser;
pub mod repl;
pub mod typecheck;

use diagnostic::Diagnostic;

pub fn parse_source(source: &str) -> Result<ast::Module, Vec<Diagnostic>> {
    let tokens = lexer::lex(source)?;
    parser::Parser::new(tokens).parse_module()
}

pub fn dump_ast(source: &str) -> Result<String, Vec<Diagnostic>> {
    let module = parse_source(source)?;
    Ok(module.dump())
}

pub fn check_source(source: &str) -> Result<(), Vec<Diagnostic>> {
    let module = parse_source(source)?;
    resolver::resolve(&module)?;
    typecheck::check(&module)
}

pub fn repl_source_for_line(line: &str) -> String {
    let trimmed = line.trim();
    if starts_with_top_level_item(trimmed) {
        trimmed.to_string()
    } else {
        format!("fn repl() {{\n  {trimmed}\n}}")
    }
}

fn starts_with_top_level_item(line: &str) -> bool {
    matches!(
        line.split_whitespace().next(),
        Some("module" | "import" | "export" | "fn" | "struct" | "enum")
    )
}
