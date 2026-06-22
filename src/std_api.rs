pub const STANDARD_IMPORTS: &[&[&str]] =
    &[&["std", "core"], &["std", "io"], &["std", "collections"]];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandardEnum {
    pub name: &'static str,
    /// Generic type parameters (`&[]` for the monomorphized legacy enums like
    /// `OptionInt`; `&["T"]` / `&["T", "E"]` for the generic `Option`/`Result`).
    pub type_params: &'static [&'static str],
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
    // Generic built-ins. `Option<T>` / `Result<T, E>` are the modern, generic
    // forms; native codegen monomorphizes them per instantiation. The payload
    // types name the type parameters, so they stay generic until used.
    StandardEnum {
        name: "Option",
        type_params: &["T"],
        variants: &[
            StandardEnumVariant {
                name: "Some",
                payload_type: Some("T"),
            },
            StandardEnumVariant {
                name: "None",
                payload_type: None,
            },
        ],
    },
    StandardEnum {
        name: "Result",
        type_params: &["T", "E"],
        variants: &[
            StandardEnumVariant {
                name: "Ok",
                payload_type: Some("T"),
            },
            StandardEnumVariant {
                name: "Err",
                payload_type: Some("E"),
            },
        ],
    },
    // Monomorphized legacy enums (predate generics; the self-hosting frontend
    // `arena_frontend.zeta` still uses `OptionInt`/`ResultInt`). Kept for
    // backward compatibility — do NOT remove (fixpoint depends on them).
    StandardEnum {
        name: "OptionInt",
        type_params: &[],
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
        type_params: &[],
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

const STD_IO_ENUMS: &[StandardEnum] = &[StandardEnum {
    name: "ResultString",
    type_params: &[],
    variants: &[
        StandardEnumVariant {
            name: "Ok",
            payload_type: Some("String"),
        },
        StandardEnumVariant {
            name: "Err",
            payload_type: Some("String"),
        },
    ],
}];

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
        name: "string_concat",
        params: &["String", "String"],
        return_type: Some("String"),
    },
    StandardFunction {
        name: "int_to_string",
        params: &["Int"],
        return_type: Some("String"),
    },
    StandardFunction {
        name: "int_abs",
        params: &["Int"],
        return_type: Some("Int"),
    },
    StandardFunction {
        name: "int_min",
        params: &["Int", "Int"],
        return_type: Some("Int"),
    },
    StandardFunction {
        name: "int_max",
        params: &["Int", "Int"],
        return_type: Some("Int"),
    },
    StandardFunction {
        name: "int_pow",
        params: &["Int", "Int"],
        return_type: Some("Int"),
    },
    StandardFunction {
        name: "string_to_int",
        params: &["String"],
        return_type: Some("Int"),
    },
    StandardFunction {
        name: "string_index_of",
        params: &["String", "String"],
        return_type: Some("Int"),
    },
    StandardFunction {
        name: "string_contains",
        params: &["String", "String"],
        return_type: Some("Bool"),
    },
    StandardFunction {
        name: "string_repeat",
        params: &["String", "Int"],
        return_type: Some("String"),
    },
    StandardFunction {
        name: "string_to_upper",
        params: &["String"],
        return_type: Some("String"),
    },
    StandardFunction {
        name: "string_to_lower",
        params: &["String"],
        return_type: Some("String"),
    },
    StandardFunction {
        name: "string_trim",
        params: &["String"],
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
    StandardFunction {
        name: "float_array_empty",
        params: &[],
        return_type: Some("FloatArray"),
    },
    StandardFunction {
        name: "float_array_push",
        params: &["FloatArray", "Float"],
        return_type: Some("FloatArray"),
    },
];

const STD_IO_FUNCTIONS: &[StandardFunction] = &[
    StandardFunction {
        name: "file_read_to_string",
        params: &["String"],
        return_type: Some("ResultString"),
    },
    StandardFunction {
        name: "path_join",
        params: &["String", "String"],
        return_type: Some("String"),
    },
    StandardFunction {
        name: "path_basename",
        params: &["String"],
        return_type: Some("String"),
    },
    StandardFunction {
        name: "diagnostic_format",
        params: &["String", "Int", "Int", "String"],
        return_type: Some("String"),
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

pub fn is_std_io_import(path: &[String]) -> bool {
    ["std", "io"]
        .into_iter()
        .eq(path.iter().map(String::as_str))
}

/// `std.collections` is a SOURCE module (HashMap/HashSet written in Zeta),
/// not an intrinsic one — importing it injects its definitions verbatim (see
/// `std_prelude`), so it carries no `core_functions`-style builtin entries.
pub fn is_std_collections_import(path: &[String]) -> bool {
    ["std", "collections"]
        .into_iter()
        .eq(path.iter().map(String::as_str))
}

pub fn core_enums() -> &'static [StandardEnum] {
    STD_CORE_ENUMS
}

pub fn io_enums() -> &'static [StandardEnum] {
    STD_IO_ENUMS
}

pub fn core_functions() -> &'static [StandardFunction] {
    STD_CORE_FUNCTIONS
}

pub fn io_functions() -> &'static [StandardFunction] {
    STD_IO_FUNCTIONS
}
