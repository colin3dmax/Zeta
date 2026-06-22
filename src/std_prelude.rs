//! Source-level standard modules: libraries written in Zeta and injected into a
//! compilation unit when imported (as opposed to the intrinsic `std.core`/
//! `std.io`, whose functions are builtins). Currently just `std.collections`
//! (generic HashMap/HashSet).
//!
//! Injection happens once, right after parsing, at every compile entry point
//! (`lib::parse_source` for single files, `module_graph::parse_sources` for the
//! multi-file graph). The injected items are ordinary AST — they flow through
//! desugar/resolve/typecheck/lower exactly like user-written code, so no other
//! stage needs to know they came from a prelude.

use crate::ast::{Item, Module};
use crate::{lexer, parser, std_api};

/// The `std.collections` library source, embedded at build time.
const COLLECTIONS_SOURCE: &str = include_str!("std/collections.zeta");

/// If `module` imports a source-level standard module, prepend that module's
/// items so its definitions precede the user code that references them. A no-op
/// for the common case of no such import — in particular the self-hosting
/// frontend imports none of these, so fixpoint is unaffected.
pub fn inject(module: &mut Module) {
    if imports(module, std_api::is_std_collections_import) {
        prepend_source(module, COLLECTIONS_SOURCE);
    }
}

fn imports(module: &Module, pred: fn(&[String]) -> bool) -> bool {
    module.items.iter().any(|item| match item {
        Item::Import { path, .. } => pred(path),
        _ => false,
    })
}

/// Parse a trusted embedded library and splice its items in front of the user's.
/// The library is part of the compiler binary, so a parse failure is a build
/// bug, not user input — hence the panic.
fn prepend_source(module: &mut Module, source: &str) {
    let tokens = lexer::lex(source).expect("embedded std module must lex");
    let prelude = parser::Parser::new(tokens)
        .parse_module()
        .expect("embedded std module must parse");
    let mut items = prelude.items;
    items.append(&mut module.items);
    module.items = items;
}
