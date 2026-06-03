pub const STANDARD_IMPORTS: &[&[&str]] = &[&["std", "core"], &["std", "io"]];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandardEnum {
    pub name: &'static str,
    pub variants: &'static [StandardEnumVariant],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandardEnumVariant {
    pub name: &'static str,
    pub payload_type: Option<&'static str>,
}

const STD_CORE_ENUMS: &[StandardEnum] = &[
    StandardEnum {
        name: "OptionInt",
        variants: &[
            StandardEnumVariant {
                name: "Some",
                payload_type: Some("Int"),
            },
            StandardEnumVariant {
                name: "None",
                payload_type: None,
            },
        ],
    },
    StandardEnum {
        name: "ResultInt",
        variants: &[
            StandardEnumVariant {
                name: "Ok",
                payload_type: Some("Int"),
            },
            StandardEnumVariant {
                name: "Err",
                payload_type: Some("String"),
            },
        ],
    },
];

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

pub fn is_std_core_import(path: &[String]) -> bool {
    ["std", "core"]
        .into_iter()
        .eq(path.iter().map(String::as_str))
}

pub fn core_enums() -> &'static [StandardEnum] {
    STD_CORE_ENUMS
}
