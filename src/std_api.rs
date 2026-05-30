pub const STANDARD_IMPORTS: &[&[&str]] = &[&["std", "core"], &["std", "io"]];

pub fn is_standard_import(path: &[String]) -> bool {
    STANDARD_IMPORTS.iter().any(|candidate| {
        candidate
            .iter()
            .copied()
            .eq(path.iter().map(String::as_str))
    })
}

pub fn standard_import_names() -> Vec<String> {
    STANDARD_IMPORTS.iter().map(|path| path.join(".")).collect()
}
