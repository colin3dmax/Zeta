//! Shared parsing for the canonical *type string* representation.
//!
//! Declared types flow through the compiler as plain `String`s (e.g. `"Int"`,
//! `"IntArray"`). Tuple types reuse that channel with a canonical surface
//! syntax `(T0, T1, ...)`, produced by the parser and decoded again at each
//! layer that maps a type string to its own type enum (`typecheck::Type`,
//! `mir::MirType`, `codegen::ZType`). This module is the single place that
//! understands the `(...)` form, so those decoders stay in sync.

/// If `name` is a tuple type string `(T0, T1, ...)`, return its element type
/// strings split at the top level (respecting nested parens). Returns `None`
/// for non-tuples and for a parenthesized single type `(T)` (which the parser
/// collapses to plain `T`), so any `Some` result has at least two elements.
pub fn tuple_parts(name: &str) -> Option<Vec<&str>> {
    let trimmed = name.trim();
    let inner = trimmed.strip_prefix('(')?.strip_suffix(')')?;
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in inner.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(inner[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = inner[start..].trim();
    if !last.is_empty() {
        parts.push(last);
    }
    if parts.len() >= 2 {
        Some(parts)
    } else {
        None
    }
}
