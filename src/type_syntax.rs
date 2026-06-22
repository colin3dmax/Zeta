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

/// If `name` is a raw pointer type string `*T`, return its pointee type string
/// `T`. Returns `None` otherwise. The pointer prefix binds outermost, so
/// `*Point`, `*(Int, Int)`, `**Int` all parse with one strip per level.
pub fn ptr_parts(name: &str) -> Option<&str> {
    let trimmed = name.trim();
    trimmed.strip_prefix('*').map(str::trim)
}

/// Split a comma-separated type list at the top level (respecting nested parens),
/// returning the trimmed element strings. An empty/blank input yields `vec![]`.
pub fn split_top_level(inner: &str) -> Vec<&str> {
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
    parts
}

/// If `name` is a generic instantiation `Base<A0, A1, ...>` (e.g. `Box<Int>`,
/// `Result<Int, String>`), return its base name and argument type strings,
/// split at the top level (respecting nested `<>` and `()`). Returns `None` for
/// non-instantiations, for tuple `(...)` / function `fn(...) -> R` forms, and
/// when the `<...>` is empty or unbalanced.
pub fn generic_parts(name: &str) -> Option<(&str, Vec<&str>)> {
    let trimmed = name.trim();
    // Tuples and function types are handled by their own decoders; never treat
    // their leading token as a generic base.
    if trimmed.starts_with('(') || trimmed.starts_with("fn") {
        return None;
    }
    let lt = trimmed.find('<')?;
    if !trimmed.ends_with('>') {
        return None;
    }
    let base = trimmed[..lt].trim();
    if base.is_empty() {
        return None;
    }
    let inner = &trimmed[lt + 1..trimmed.len() - 1];
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (i, c) in inner.char_indices() {
        match c {
            '<' | '(' => depth += 1,
            '>' | ')' => depth -= 1,
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
    if parts.is_empty() {
        return None;
    }
    Some((base, parts))
}

/// The base name of a (possibly generic) type string: `Box<Int>` → `Box`,
/// `Point` → `Point`. Tuple/function forms are returned unchanged.
pub fn base_name(name: &str) -> &str {
    match generic_parts(name) {
        Some((base, _)) => base,
        None => name.trim(),
    }
}

/// The mangled free-function name a trait method lowers to for a given
/// implementing type. UFCS dispatch routes a call `method(recv, ..)` to
/// `dispatch_name(method, base_name(recv_type))`. Mirrors the `$` convention
/// generic monomorphization uses (`id$Int`); the single source of truth shared
/// by the impl flattener (desugar) and the per-backend dispatchers
/// (runtime / codegen).
pub fn dispatch_name(method: &str, target_base: &str) -> String {
    format!("{method}${target_base}")
}

/// If `name` is a function type string `fn(P0, P1, ...) -> R`, return its
/// parameter type strings and return type string. Returns `None` otherwise.
pub fn fn_parts(name: &str) -> Option<(Vec<&str>, &str)> {
    let rest = name.trim().strip_prefix("fn")?.trim_start();
    let rest = rest.strip_prefix('(')?;
    // Find the matching `)` for the parameter list at depth 0.
    let mut depth = 0i32;
    let mut close = None;
    for (i, c) in rest.char_indices() {
        match c {
            '(' => depth += 1,
            ')' if depth == 0 => {
                close = Some(i);
                break;
            }
            ')' => depth -= 1,
            _ => {}
        }
    }
    let close = close?;
    let params = split_top_level(rest[..close].trim());
    let after = rest[close + 1..].trim();
    let ret = after.strip_prefix("->")?.trim();
    if ret.is_empty() {
        return None;
    }
    Some((params, ret))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_parts_basic() {
        assert_eq!(generic_parts("Box<Int>"), Some(("Box", vec!["Int"])));
        assert_eq!(
            generic_parts("Result<Int, String>"),
            Some(("Result", vec!["Int", "String"]))
        );
    }

    #[test]
    fn generic_parts_nested() {
        // Commas inside nested `<>` / `()` must not split the top level.
        assert_eq!(
            generic_parts("Map<Box<Int>, Pair<A, B>>"),
            Some(("Map", vec!["Box<Int>", "Pair<A, B>"]))
        );
        assert_eq!(
            generic_parts("Box<(Int, String)>"),
            Some(("Box", vec!["(Int, String)"]))
        );
    }

    #[test]
    fn generic_parts_rejects_non_generics() {
        assert_eq!(generic_parts("Int"), None);
        assert_eq!(generic_parts("Point"), None);
        assert_eq!(generic_parts("(Int, String)"), None);
        assert_eq!(generic_parts("fn(Int) -> Bool"), None);
        assert_eq!(generic_parts("Box<>"), None);
    }

    #[test]
    fn base_name_strips_args() {
        assert_eq!(base_name("Box<Int>"), "Box");
        assert_eq!(base_name("Result<Int, String>"), "Result");
        assert_eq!(base_name("Point"), "Point");
    }
}
