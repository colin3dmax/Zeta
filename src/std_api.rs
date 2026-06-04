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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandardFunction {
    pub name: &'static str,
    pub params: &'static [&'static str],
    pub return_type: Option<&'static str>,
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

const STD_CORE_FUNCTIONS: &[StandardFunction] = &[
    StandardFunction {
        name: "string_len",
        params: &["String"],
        return_type: Some("Int"),
    },
    StandardFunction {
        name: "string_byte_at",
        params: &["String", "Int"],
        return_type: Some("Int"),
    },
    StandardFunction {
        name: "string_byte_slice",
        params: &["String", "Int", "Int"],
        return_type: Some("String"),
    },
    StandardFunction {
        name: "ascii_is_digit",
        params: &["Int"],
        return_type: Some("Bool"),
    },
    StandardFunction {
        name: "ascii_is_alpha",
        params: &["Int"],
        return_type: Some("Bool"),
    },
    StandardFunction {
        name: "ascii_is_alnum",
        params: &["Int"],
        return_type: Some("Bool"),
    },
    StandardFunction {
        name: "ascii_is_whitespace",
        params: &["Int"],
        return_type: Some("Bool"),
    },
    StandardFunction {
        name: "int_array_empty",
        params: &[],
        return_type: Some("IntArray"),
    },
    StandardFunction {
        name: "int_array_push",
        params: &["IntArray", "Int"],
        return_type: Some("IntArray"),
    },
    StandardFunction {
        name: "string_array_empty",
        params: &[],
        return_type: Some("StringArray"),
    },
    StandardFunction {
        name: "string_array_push",
        params: &["StringArray", "String"],
        return_type: Some("StringArray"),
    },
    StandardFunction {
        name: "bool_array_empty",
        params: &[],
        return_type: Some("BoolArray"),
    },
    StandardFunction {
        name: "bool_array_push",
        params: &["BoolArray", "Bool"],
        return_type: Some("BoolArray"),
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

pub fn core_functions() -> &'static [StandardFunction] {
    STD_CORE_FUNCTIONS
}
