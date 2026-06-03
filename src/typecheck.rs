use crate::ast::{BinaryOp, Expr, Function, Item, Module, Pattern, Stmt, UnaryOp};
use crate::diagnostic::{Diagnostic, Span};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Type {
    Int,
    String,
    Bool,
    Named(String),
    Unit,
    Error,
}

pub fn check(module: &Module) -> Result<(), Vec<Diagnostic>> {
    check_with_external_functions(module, &[])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalFunction {
    pub name: String,
    pub params: Vec<String>,
    pub return_type: Option<String>,
    pub target_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalStruct {
    pub name: String,
    pub fields: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalEnum {
    pub name: String,
    pub variants: Vec<(String, Option<String>)>,
}

pub fn check_with_external_functions(
    module: &Module,
    external_functions: &[ExternalFunction],
) -> Result<(), Vec<Diagnostic>> {
    check_with_external_items(module, external_functions, &[], &[])
}

pub fn check_with_external_items(
    module: &Module,
    external_functions: &[ExternalFunction],
    external_structs: &[ExternalStruct],
    external_enums: &[ExternalEnum],
) -> Result<(), Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    let mut functions = function_signatures(module);
    for function in external_functions {
        functions
            .entry(function.name.clone())
            .or_insert_with(|| external_function_signature(function));
    }
    let mut structs = struct_types(module);
    for external_struct in external_structs {
        structs
            .entry(external_struct.name.clone())
            .or_insert_with(|| external_struct_type(external_struct));
    }
    let mut enums = enum_types(module);
    for external_enum in external_enums {
        enums
            .entry(external_enum.name.clone())
            .or_insert_with(|| external_enum_type(external_enum));
    }
    for item in &module.items {
        if let Item::Function(function) = item {
            check_function(function, &functions, &structs, &enums, &mut diagnostics);
        }
    }

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

fn external_function_signature(function: &ExternalFunction) -> FunctionSignature {
    FunctionSignature {
        params: function
            .params
            .iter()
            .map(|param| parse_type(param))
            .collect(),
        return_type: function
            .return_type
            .as_deref()
            .map(parse_type)
            .unwrap_or(Type::Unit),
    }
}

fn external_struct_type(external_struct: &ExternalStruct) -> StructType {
    StructType {
        fields: external_struct
            .fields
            .iter()
            .map(|(name, ty)| (name.clone(), parse_type(ty)))
            .collect(),
    }
}

fn external_enum_type(external_enum: &ExternalEnum) -> EnumType {
    EnumType {
        variants: external_enum
            .variants
            .iter()
            .map(|(name, payload_type)| (name.clone(), payload_type.as_deref().map(parse_type)))
            .collect(),
    }
}

fn check_function(
    function: &Function,
    functions: &HashMap<String, FunctionSignature>,
    structs: &HashMap<String, StructType>,
    enums: &HashMap<String, EnumType>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut locals = HashMap::new();
    for param in &function.params {
        locals.insert(
            param.name.clone(),
            Binding {
                ty: parse_type(&param.ty),
                mutable: false,
            },
        );
    }
    let return_type = function
        .return_type
        .as_deref()
        .map(parse_type)
        .unwrap_or(Type::Unit);
    check_stmts(
        &function.body,
        &mut locals,
        functions,
        structs,
        enums,
        &return_type,
        &function.name,
        diagnostics,
    );
}

fn check_stmts(
    stmts: &[Stmt],
    locals: &mut HashMap<String, Binding>,
    functions: &HashMap<String, FunctionSignature>,
    structs: &HashMap<String, StructType>,
    enums: &HashMap<String, EnumType>,
    return_type: &Type,
    function_name: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for stmt in stmts {
        match stmt {
            Stmt::Let {
                mutable,
                name,
                ty,
                value,
                ..
            } => {
                let value_type = infer_expr(value, locals, functions, structs, enums, diagnostics);
                let declared_type = ty.as_deref().map(parse_type);
                if let Some(declared_type) = declared_type {
                    expect_type(
                        &value_type,
                        &declared_type,
                        "TYPE_LET_MISMATCH",
                        value.span(),
                        diagnostics,
                    );
                    locals.insert(
                        name.clone(),
                        Binding {
                            ty: declared_type,
                            mutable: *mutable,
                        },
                    );
                } else {
                    locals.insert(
                        name.clone(),
                        Binding {
                            ty: value_type,
                            mutable: *mutable,
                        },
                    );
                }
            }
            Stmt::Assign {
                name,
                name_span,
                value,
            } => {
                let value_type = infer_expr(value, locals, functions, structs, enums, diagnostics);
                match locals.get(name) {
                    Some(binding) if binding.mutable => {
                        expect_type(
                            &value_type,
                            &binding.ty,
                            "TYPE_ASSIGN_MISMATCH",
                            value.span(),
                            diagnostics,
                        );
                    }
                    Some(_) => diagnostics.push(Diagnostic::new(
                        "TYPE_ASSIGN_IMMUTABLE",
                        format!(
                            "cannot assign to immutable local `{name}`; declare it with `let mut`"
                        ),
                        *name_span,
                    )),
                    None => diagnostics.push(Diagnostic::new(
                        "TYPE_UNKNOWN_NAME",
                        format!("unknown name `{name}`"),
                        *name_span,
                    )),
                }
            }
            Stmt::If {
                condition,
                then_body,
                else_body,
            } => {
                let condition_type =
                    infer_expr(condition, locals, functions, structs, enums, diagnostics);
                expect_type(
                    &condition_type,
                    &Type::Bool,
                    "TYPE_IF_CONDITION",
                    condition.span(),
                    diagnostics,
                );
                let mut then_locals = locals.clone();
                check_stmts(
                    then_body,
                    &mut then_locals,
                    functions,
                    structs,
                    enums,
                    return_type,
                    function_name,
                    diagnostics,
                );
                let mut else_locals = locals.clone();
                check_stmts(
                    else_body,
                    &mut else_locals,
                    functions,
                    structs,
                    enums,
                    return_type,
                    function_name,
                    diagnostics,
                );
            }
            Stmt::While { condition, body } => {
                let condition_type =
                    infer_expr(condition, locals, functions, structs, enums, diagnostics);
                expect_type(
                    &condition_type,
                    &Type::Bool,
                    "TYPE_WHILE_CONDITION",
                    condition.span(),
                    diagnostics,
                );
                let mut loop_locals = locals.clone();
                check_stmts(
                    body,
                    &mut loop_locals,
                    functions,
                    structs,
                    enums,
                    return_type,
                    function_name,
                    diagnostics,
                );
            }
            Stmt::Match { value, arms } => {
                let value_type = infer_expr(value, locals, functions, structs, enums, diagnostics);
                for arm in arms {
                    let mut arm_locals = locals.clone();
                    for (name, ty) in
                        check_pattern(&arm.pattern, &value_type, value.span(), enums, diagnostics)
                    {
                        arm_locals.insert(name, Binding { ty, mutable: false });
                    }
                    check_stmts(
                        &arm.body,
                        &mut arm_locals,
                        functions,
                        structs,
                        enums,
                        return_type,
                        function_name,
                        diagnostics,
                    );
                }
                check_match_exhaustiveness(&value_type, value.span(), arms, enums, diagnostics);
            }
            Stmt::Return(Some(value)) => {
                let value_type = infer_expr(value, locals, functions, structs, enums, diagnostics);
                let code = if function_name.is_empty() {
                    "TYPE_RETURN_MISMATCH"
                } else {
                    "TYPE_RETURN_MISMATCH"
                };
                expect_type(&value_type, return_type, code, value.span(), diagnostics);
            }
            Stmt::Return(None) => {
                expect_type(
                    &Type::Unit,
                    return_type,
                    "TYPE_RETURN_MISMATCH",
                    Span::new(0, 0),
                    diagnostics,
                );
            }
            Stmt::Expr(value) => {
                let _ = infer_expr(value, locals, functions, structs, enums, diagnostics);
            }
        }
    }
}

fn check_match_exhaustiveness(
    value_type: &Type,
    value_span: Span,
    arms: &[crate::ast::MatchArm],
    enums: &HashMap<String, EnumType>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if arms
        .iter()
        .any(|arm| matches!(arm.pattern, Pattern::Wildcard | Pattern::Name(_)))
    {
        return;
    }

    match value_type {
        Type::Bool => {
            let covered = arms
                .iter()
                .filter_map(|arm| match arm.pattern {
                    Pattern::Bool(value) => Some(value),
                    _ => None,
                })
                .collect::<HashSet<_>>();
            if !covered.contains(&true) || !covered.contains(&false) {
                diagnostics.push(Diagnostic::new(
                    "TYPE_MATCH_NON_EXHAUSTIVE",
                    "Bool match must cover both true and false or include `_`",
                    value_span,
                ));
            }
        }
        Type::Named(enum_name) => {
            let Some(enum_type) = enums.get(enum_name) else {
                return;
            };
            let covered = arms
                .iter()
                .filter_map(|arm| match &arm.pattern {
                    Pattern::Variant {
                        enum_name: pattern_enum,
                        variant,
                        ..
                    } if pattern_enum == enum_name => Some(variant.as_str()),
                    _ => None,
                })
                .collect::<HashSet<_>>();
            if !enum_type
                .variants
                .keys()
                .all(|variant| covered.contains(variant.as_str()))
            {
                diagnostics.push(Diagnostic::new(
                    "TYPE_MATCH_NON_EXHAUSTIVE",
                    format!("match on `{enum_name}` must cover every variant or include `_`"),
                    value_span,
                ));
            }
        }
        Type::Int | Type::String | Type::Unit | Type::Error => {}
    }
}

fn check_pattern(
    pattern: &Pattern,
    value_type: &Type,
    value_span: Span,
    enums: &HashMap<String, EnumType>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<(String, Type)> {
    match pattern {
        Pattern::Variant {
            enum_name,
            variant,
            binding,
        } => {
            expect_type(
                value_type,
                &Type::Named(enum_name.clone()),
                "TYPE_MATCH_PATTERN",
                value_span,
                diagnostics,
            );
            match enums.get(enum_name) {
                Some(enum_type) => match enum_type.variants.get(variant) {
                    Some(payload_type) => match (payload_type, binding) {
                        (Some(payload_type), Some(binding)) => {
                            return vec![(binding.clone(), payload_type.clone())];
                        }
                        (Some(_), None) => diagnostics.push(Diagnostic::new(
                            "TYPE_ENUM_PATTERN_ARITY",
                            format!(
                                "variant `{enum_name}.{variant}` carries a payload and must bind it"
                            ),
                            value_span,
                        )),
                        (None, Some(_)) => diagnostics.push(Diagnostic::new(
                            "TYPE_ENUM_PATTERN_ARITY",
                            format!("variant `{enum_name}.{variant}` does not carry a payload"),
                            value_span,
                        )),
                        (None, None) => {}
                    },
                    None => diagnostics.push(Diagnostic::new(
                        "TYPE_UNKNOWN_VARIANT",
                        format!("unknown variant `{variant}` on enum `{enum_name}`"),
                        value_span,
                    )),
                },
                None => diagnostics.push(Diagnostic::new(
                    "TYPE_UNKNOWN_ENUM",
                    format!("unknown enum `{enum_name}`"),
                    value_span,
                )),
            }
        }
        Pattern::Int(_) => expect_type(
            value_type,
            &Type::Int,
            "TYPE_MATCH_PATTERN",
            value_span,
            diagnostics,
        ),
        Pattern::String(_) => expect_type(
            value_type,
            &Type::String,
            "TYPE_MATCH_PATTERN",
            value_span,
            diagnostics,
        ),
        Pattern::Bool(_) => expect_type(
            value_type,
            &Type::Bool,
            "TYPE_MATCH_PATTERN",
            value_span,
            diagnostics,
        ),
        Pattern::Name(name) => return vec![(name.clone(), value_type.clone())],
        Pattern::Wildcard => {}
    }
    Vec::new()
}

fn infer_expr(
    expr: &Expr,
    locals: &HashMap<String, Binding>,
    functions: &HashMap<String, FunctionSignature>,
    structs: &HashMap<String, StructType>,
    enums: &HashMap<String, EnumType>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Type {
    match expr {
        Expr::Name { name, .. } => locals
            .get(name)
            .map(|binding| binding.ty.clone())
            .unwrap_or(Type::Error),
        Expr::Int { .. } => Type::Int,
        Expr::String { .. } => Type::String,
        Expr::Bool { .. } => Type::Bool,
        Expr::Binary {
            op, left, right, ..
        } => {
            let left_type = infer_expr(left, locals, functions, structs, enums, diagnostics);
            let right_type = infer_expr(right, locals, functions, structs, enums, diagnostics);
            match op {
                BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                    expect_type(
                        &left_type,
                        &Type::Int,
                        "TYPE_BINARY_OPERAND",
                        left.span(),
                        diagnostics,
                    );
                    expect_type(
                        &right_type,
                        &Type::Int,
                        "TYPE_BINARY_OPERAND",
                        right.span(),
                        diagnostics,
                    );
                    Type::Int
                }
                BinaryOp::And | BinaryOp::Or => {
                    expect_type(
                        &left_type,
                        &Type::Bool,
                        "TYPE_LOGICAL_OPERAND",
                        left.span(),
                        diagnostics,
                    );
                    expect_type(
                        &right_type,
                        &Type::Bool,
                        "TYPE_LOGICAL_OPERAND",
                        right.span(),
                        diagnostics,
                    );
                    Type::Bool
                }
                BinaryOp::Eq | BinaryOp::NotEq => {
                    expect_type(
                        &right_type,
                        &left_type,
                        "TYPE_EQUALITY_OPERAND",
                        right.span(),
                        diagnostics,
                    );
                    Type::Bool
                }
                BinaryOp::Lt | BinaryOp::Lte | BinaryOp::Gt | BinaryOp::Gte => {
                    expect_type(
                        &left_type,
                        &Type::Int,
                        "TYPE_ORDERING_OPERAND",
                        left.span(),
                        diagnostics,
                    );
                    expect_type(
                        &right_type,
                        &Type::Int,
                        "TYPE_ORDERING_OPERAND",
                        right.span(),
                        diagnostics,
                    );
                    Type::Bool
                }
            }
        }
        Expr::Unary { op, expr, .. } => {
            let expr_type = infer_expr(expr, locals, functions, structs, enums, diagnostics);
            match op {
                UnaryOp::Not => {
                    expect_type(
                        &expr_type,
                        &Type::Bool,
                        "TYPE_UNARY_OPERAND",
                        expr.span(),
                        diagnostics,
                    );
                    Type::Bool
                }
            }
        }
        Expr::Call {
            callee,
            callee_span,
            args,
            ..
        } => {
            if let Some((enum_name, variant)) = callee.rsplit_once('.') {
                if let Some(enum_type) = enums.get(enum_name) {
                    return infer_enum_variant_call(
                        enum_name,
                        variant,
                        *callee_span,
                        args,
                        locals,
                        functions,
                        structs,
                        enums,
                        enum_type,
                        diagnostics,
                    );
                }
            }
            let Some(signature) = functions.get(callee) else {
                return Type::Error;
            };
            if args.len() != signature.params.len() {
                diagnostics.push(Diagnostic::new(
                    "TYPE_CALL_ARITY",
                    format!(
                        "function `{callee}` expects {} arguments, found {}",
                        signature.params.len(),
                        args.len()
                    ),
                    *callee_span,
                ));
                return signature.return_type.clone();
            }
            for (arg, expected) in args.iter().zip(&signature.params) {
                let found = infer_expr(arg, locals, functions, structs, enums, diagnostics);
                expect_type(
                    &found,
                    expected,
                    "TYPE_CALL_ARGUMENT",
                    arg.span(),
                    diagnostics,
                );
            }
            signature.return_type.clone()
        }
        Expr::StructLiteral {
            ty,
            ty_span,
            fields,
            ..
        } => {
            let Some(struct_type) = structs.get(ty) else {
                diagnostics.push(Diagnostic::new(
                    "TYPE_UNKNOWN_STRUCT",
                    format!("unknown struct `{ty}`"),
                    *ty_span,
                ));
                for field in fields {
                    let _ =
                        infer_expr(&field.value, locals, functions, structs, enums, diagnostics);
                }
                return Type::Named(ty.clone());
            };

            let mut seen = HashMap::new();
            for field in fields {
                if seen.insert(field.name.clone(), field.name_span).is_some() {
                    diagnostics.push(Diagnostic::new(
                        "TYPE_DUPLICATE_FIELD",
                        format!("duplicate field `{}` in `{ty}` literal", field.name),
                        field.name_span,
                    ));
                }
                let found =
                    infer_expr(&field.value, locals, functions, structs, enums, diagnostics);
                match struct_type.fields.get(&field.name) {
                    Some(expected) => {
                        expect_type(
                            &found,
                            expected,
                            "TYPE_STRUCT_FIELD",
                            field.value.span(),
                            diagnostics,
                        );
                    }
                    None => diagnostics.push(Diagnostic::new(
                        "TYPE_UNKNOWN_FIELD",
                        format!("unknown field `{}` on struct `{ty}`", field.name),
                        field.name_span,
                    )),
                }
            }
            for field in struct_type.fields.keys() {
                if !seen.contains_key(field) {
                    diagnostics.push(Diagnostic::new(
                        "TYPE_MISSING_FIELD",
                        format!("missing field `{field}` in `{ty}` literal"),
                        *ty_span,
                    ));
                }
            }
            Type::Named(ty.clone())
        }
        Expr::FieldAccess {
            base,
            field,
            field_span,
            ..
        } => {
            if let Expr::Name {
                name: enum_name, ..
            } = base.as_ref()
            {
                if !locals.contains_key(enum_name) {
                    match enums.get(enum_name) {
                        Some(enum_type) => {
                            if let Some(payload_type) = enum_type.variants.get(field) {
                                if payload_type.is_some() {
                                    diagnostics.push(Diagnostic::new(
                                        "TYPE_ENUM_VARIANT_ARITY",
                                        format!(
                                            "variant `{enum_name}.{field}` carries a payload and must be called"
                                        ),
                                        *field_span,
                                    ));
                                }
                                return Type::Named(enum_name.clone());
                            } else {
                                diagnostics.push(Diagnostic::new(
                                    "TYPE_UNKNOWN_VARIANT",
                                    format!("unknown variant `{field}` on enum `{enum_name}`"),
                                    *field_span,
                                ));
                                return Type::Named(enum_name.clone());
                            }
                        }
                        None => {}
                    }
                }
            }
            let base_type = infer_expr(base, locals, functions, structs, enums, diagnostics);
            let Type::Named(struct_name) = base_type else {
                diagnostics.push(Diagnostic::new(
                    "TYPE_FIELD_BASE",
                    "field access requires a struct value",
                    base.span(),
                ));
                return Type::Named(field.clone());
            };
            let Some(struct_type) = structs.get(&struct_name) else {
                diagnostics.push(Diagnostic::new(
                    "TYPE_UNKNOWN_STRUCT",
                    format!("unknown struct `{struct_name}`"),
                    base.span(),
                ));
                return Type::Named(field.clone());
            };
            struct_type.fields.get(field).cloned().unwrap_or_else(|| {
                diagnostics.push(Diagnostic::new(
                    "TYPE_UNKNOWN_FIELD",
                    format!("unknown field `{field}` on struct `{struct_name}`"),
                    *field_span,
                ));
                Type::Named(field.clone())
            })
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn infer_enum_variant_call(
    enum_name: &str,
    variant: &str,
    callee_span: Span,
    args: &[Expr],
    locals: &HashMap<String, Binding>,
    functions: &HashMap<String, FunctionSignature>,
    structs: &HashMap<String, StructType>,
    enums: &HashMap<String, EnumType>,
    enum_type: &EnumType,
    diagnostics: &mut Vec<Diagnostic>,
) -> Type {
    match enum_type.variants.get(variant) {
        Some(Some(payload_type)) => {
            if args.len() != 1 {
                diagnostics.push(Diagnostic::new(
                    "TYPE_ENUM_VARIANT_ARITY",
                    format!(
                        "variant `{enum_name}.{variant}` expects 1 payload argument, found {}",
                        args.len()
                    ),
                    callee_span,
                ));
            }
            for arg in args {
                let found = infer_expr(arg, locals, functions, structs, enums, diagnostics);
                expect_type(
                    &found,
                    payload_type,
                    "TYPE_ENUM_VARIANT_PAYLOAD",
                    arg.span(),
                    diagnostics,
                );
            }
        }
        Some(None) => {
            if !args.is_empty() {
                diagnostics.push(Diagnostic::new(
                    "TYPE_ENUM_VARIANT_ARITY",
                    format!(
                        "variant `{enum_name}.{variant}` expects no payload arguments, found {}",
                        args.len()
                    ),
                    callee_span,
                ));
            }
            for arg in args {
                let _ = infer_expr(arg, locals, functions, structs, enums, diagnostics);
            }
        }
        None => diagnostics.push(Diagnostic::new(
            "TYPE_UNKNOWN_VARIANT",
            format!("unknown variant `{variant}` on enum `{enum_name}`"),
            callee_span,
        )),
    }
    Type::Named(enum_name.to_string())
}

#[derive(Debug, Clone)]
struct FunctionSignature {
    params: Vec<Type>,
    return_type: Type,
}

#[derive(Debug, Clone)]
struct StructType {
    fields: HashMap<String, Type>,
}

#[derive(Debug, Clone)]
struct EnumType {
    variants: HashMap<String, Option<Type>>,
}

#[derive(Debug, Clone)]
struct Binding {
    ty: Type,
    mutable: bool,
}

fn function_signatures(module: &Module) -> HashMap<String, FunctionSignature> {
    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Function(function) => Some((
                function.name.clone(),
                FunctionSignature {
                    params: function
                        .params
                        .iter()
                        .map(|param| parse_type(&param.ty))
                        .collect(),
                    return_type: function
                        .return_type
                        .as_deref()
                        .map(parse_type)
                        .unwrap_or(Type::Unit),
                },
            )),
            _ => None,
        })
        .collect()
}

fn struct_types(module: &Module) -> HashMap<String, StructType> {
    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(decl) => Some((
                decl.name.clone(),
                StructType {
                    fields: decl
                        .fields
                        .iter()
                        .map(|field| (field.name.clone(), parse_type(&field.ty)))
                        .collect(),
                },
            )),
            _ => None,
        })
        .collect()
}

fn enum_types(module: &Module) -> HashMap<String, EnumType> {
    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Enum(decl) => Some((
                decl.name.clone(),
                EnumType {
                    variants: decl
                        .variants
                        .iter()
                        .map(|variant| {
                            (
                                variant.name.clone(),
                                variant.payload_type.as_deref().map(parse_type),
                            )
                        })
                        .collect(),
                },
            )),
            _ => None,
        })
        .collect()
}

fn parse_type(name: &str) -> Type {
    match name {
        "Int" => Type::Int,
        "String" => Type::String,
        "Bool" => Type::Bool,
        other => Type::Named(other.to_string()),
    }
}

fn expect_type(
    found: &Type,
    expected: &Type,
    code: &'static str,
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if found == expected {
        return;
    }
    if matches!(found, Type::Error) || matches!(expected, Type::Error) {
        return;
    }
    diagnostics.push(Diagnostic::new(
        code,
        format!("expected {}, found {}", expected.display(), found.display()),
        span,
    ));
}

impl Type {
    fn display(&self) -> String {
        match self {
            Type::Int => "Int".to_string(),
            Type::String => "String".to_string(),
            Type::Bool => "Bool".to_string(),
            Type::Named(name) => name.clone(),
            Type::Unit => "Unit".to_string(),
            Type::Error => "<error>".to_string(),
        }
    }
}
