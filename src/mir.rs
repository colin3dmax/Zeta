use crate::ast::{BinaryOp, Expr, Function, Item, Module, Param, Pattern, Stmt, UnaryOp};
use crate::diagnostic::{Diagnostic, Span};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub enums: Vec<MirEnum>,
    pub functions: Vec<MirFunction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirEnum {
    pub name: String,
    pub variants: Vec<MirEnumVariant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirEnumVariant {
    pub name: String,
    pub payload_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirFunction {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Option<String>,
    pub body: Vec<MirStmt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirStmt {
    Local {
        mutable: bool,
        name: String,
        ty: Option<String>,
        value: MirExpr,
    },
    Store {
        name: String,
        value: MirExpr,
    },
    If {
        condition: MirExpr,
        then_body: Vec<MirStmt>,
        else_body: Vec<MirStmt>,
    },
    While {
        condition: MirExpr,
        body: Vec<MirStmt>,
    },
    Match {
        value: MirExpr,
        arms: Vec<MirMatchArm>,
    },
    Return(Option<MirExpr>),
    Drop(MirExpr),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirMatchArm {
    pub pattern: MirPattern,
    pub body: Vec<MirStmt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirPattern {
    Name(String),
    Variant {
        enum_name: String,
        variant: String,
        binding: Option<String>,
    },
    Int(String),
    String(String),
    Bool(bool),
    Wildcard,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirExpr {
    Load(String),
    Int(String),
    String(String),
    Bool(bool),
    Binary {
        op: BinaryOp,
        left: Box<MirExpr>,
        right: Box<MirExpr>,
    },
    Unary {
        op: UnaryOp,
        expr: Box<MirExpr>,
    },
    Call {
        callee: String,
        args: Vec<MirExpr>,
    },
    EnumVariant {
        enum_name: String,
        variant: String,
        payload: Option<Box<MirExpr>>,
    },
    StructLiteral {
        ty: String,
        fields: Vec<MirStructField>,
    },
    FieldAccess {
        base: Box<MirExpr>,
        field: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirStructField {
    pub name: String,
    pub value: MirExpr,
}

pub fn lower(module: &Module) -> Program {
    let enum_variants = enum_variants(module);
    Program {
        enums: module
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Enum(decl) => Some(MirEnum {
                    name: decl.name.clone(),
                    variants: decl
                        .variants
                        .iter()
                        .map(|variant| MirEnumVariant {
                            name: variant.name.clone(),
                            payload_type: variant.payload_type.clone(),
                        })
                        .collect(),
                }),
                _ => None,
            })
            .collect(),
        functions: module
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Function(function) => Some(lower_function(function, &enum_variants)),
                _ => None,
            })
            .collect(),
    }
}

pub fn dump(module: &Module) -> String {
    dump_program(&lower(module))
}

pub fn dump_program(program: &Program) -> String {
    let mut out = String::from("MirModule\n");
    for enum_decl in &program.enums {
        out.push_str(&format!(
            "  enum {} variants={}\n",
            enum_decl.name,
            enum_decl
                .variants
                .iter()
                .map(|variant| match &variant.payload_type {
                    Some(payload_type) => format!("{}({payload_type})", variant.name),
                    None => variant.name.clone(),
                })
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    for function in &program.functions {
        dump_function(function, 1, &mut out);
    }
    out
}

pub fn verify(program: &Program) -> Result<(), Vec<Diagnostic>> {
    let mut verifier = MirVerifier::new(program);
    verifier.verify_program();
    if verifier.diagnostics.is_empty() {
        Ok(())
    } else {
        Err(verifier.diagnostics)
    }
}

fn lower_function(
    function: &Function,
    enum_variants: &HashMap<String, HashMap<String, Option<String>>>,
) -> MirFunction {
    MirFunction {
        name: function.name.clone(),
        params: function.params.clone(),
        return_type: function.return_type.clone(),
        body: function
            .body
            .iter()
            .map(|stmt| lower_stmt(stmt, enum_variants))
            .collect(),
    }
}

fn lower_stmt(
    stmt: &Stmt,
    enum_variants: &HashMap<String, HashMap<String, Option<String>>>,
) -> MirStmt {
    match stmt {
        Stmt::Let {
            mutable,
            name,
            ty,
            value,
            ..
        } => MirStmt::Local {
            mutable: *mutable,
            name: name.clone(),
            ty: ty.clone(),
            value: lower_expr(value, enum_variants),
        },
        Stmt::Assign { name, value, .. } => MirStmt::Store {
            name: name.clone(),
            value: lower_expr(value, enum_variants),
        },
        Stmt::If {
            condition,
            then_body,
            else_body,
        } => MirStmt::If {
            condition: lower_expr(condition, enum_variants),
            then_body: then_body
                .iter()
                .map(|stmt| lower_stmt(stmt, enum_variants))
                .collect(),
            else_body: else_body
                .iter()
                .map(|stmt| lower_stmt(stmt, enum_variants))
                .collect(),
        },
        Stmt::While { condition, body } => MirStmt::While {
            condition: lower_expr(condition, enum_variants),
            body: body
                .iter()
                .map(|stmt| lower_stmt(stmt, enum_variants))
                .collect(),
        },
        Stmt::Match { value, arms } => MirStmt::Match {
            value: lower_expr(value, enum_variants),
            arms: arms
                .iter()
                .map(|arm| MirMatchArm {
                    pattern: lower_pattern(&arm.pattern),
                    body: arm
                        .body
                        .iter()
                        .map(|stmt| lower_stmt(stmt, enum_variants))
                        .collect(),
                })
                .collect(),
        },
        Stmt::Return(value) => {
            MirStmt::Return(value.as_ref().map(|expr| lower_expr(expr, enum_variants)))
        }
        Stmt::Expr(value) => MirStmt::Drop(lower_expr(value, enum_variants)),
    }
}

fn lower_pattern(pattern: &Pattern) -> MirPattern {
    match pattern {
        Pattern::Name(name) => MirPattern::Name(name.clone()),
        Pattern::Variant {
            enum_name,
            variant,
            binding,
        } => MirPattern::Variant {
            enum_name: enum_name.clone(),
            variant: variant.clone(),
            binding: binding.clone(),
        },
        Pattern::Int(value) => MirPattern::Int(value.clone()),
        Pattern::String(value) => MirPattern::String(value.clone()),
        Pattern::Bool(value) => MirPattern::Bool(*value),
        Pattern::Wildcard => MirPattern::Wildcard,
    }
}

fn lower_expr(
    expr: &Expr,
    enum_variants: &HashMap<String, HashMap<String, Option<String>>>,
) -> MirExpr {
    match expr {
        Expr::Name { name, .. } => MirExpr::Load(name.clone()),
        Expr::Int { value, .. } => MirExpr::Int(value.clone()),
        Expr::String { value, .. } => MirExpr::String(value.clone()),
        Expr::Bool { value, .. } => MirExpr::Bool(*value),
        Expr::Binary {
            op, left, right, ..
        } => MirExpr::Binary {
            op: *op,
            left: Box::new(lower_expr(left, enum_variants)),
            right: Box::new(lower_expr(right, enum_variants)),
        },
        Expr::Unary { op, expr, .. } => MirExpr::Unary {
            op: *op,
            expr: Box::new(lower_expr(expr, enum_variants)),
        },
        Expr::Call { callee, args, .. } => {
            if let Some((enum_name, variant)) = callee.rsplit_once('.') {
                if enum_variants
                    .get(enum_name)
                    .is_some_and(|variants| variants.contains_key(variant))
                {
                    return MirExpr::EnumVariant {
                        enum_name: enum_name.to_string(),
                        variant: variant.to_string(),
                        payload: args
                            .first()
                            .map(|arg| Box::new(lower_expr(arg, enum_variants))),
                    };
                }
            }
            MirExpr::Call {
                callee: callee.clone(),
                args: args
                    .iter()
                    .map(|arg| lower_expr(arg, enum_variants))
                    .collect(),
            }
        }
        Expr::StructLiteral { ty, fields, .. } => MirExpr::StructLiteral {
            ty: ty.clone(),
            fields: fields
                .iter()
                .map(|field| MirStructField {
                    name: field.name.clone(),
                    value: lower_expr(&field.value, enum_variants),
                })
                .collect(),
        },
        Expr::FieldAccess { base, field, .. } => {
            if let Expr::Name {
                name: enum_name, ..
            } = base.as_ref()
            {
                if enum_variants
                    .get(enum_name)
                    .is_some_and(|variants| variants.contains_key(field))
                {
                    return MirExpr::EnumVariant {
                        enum_name: enum_name.clone(),
                        variant: field.clone(),
                        payload: None,
                    };
                }
            }
            MirExpr::FieldAccess {
                base: Box::new(lower_expr(base, enum_variants)),
                field: field.clone(),
            }
        }
    }
}

fn enum_variants(module: &Module) -> HashMap<String, HashMap<String, Option<String>>> {
    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Enum(decl) => Some((
                decl.name.clone(),
                decl.variants
                    .iter()
                    .map(|variant| (variant.name.clone(), variant.payload_type.clone()))
                    .collect(),
            )),
            _ => None,
        })
        .collect()
}

fn dump_function(function: &MirFunction, indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    let return_type = function.return_type.as_deref().unwrap_or("Unit");
    out.push_str(&format!("{pad}fn {} -> {return_type}\n", function.name));
    for param in &function.params {
        out.push_str(&format!("{pad}  param {}: {}\n", param.name, param.ty));
    }
    out.push_str(&format!("{pad}  block entry\n"));
    let mut ctx = DumpCtx::default();
    for stmt in &function.body {
        ctx.dump_stmt(stmt, indent + 2, out);
    }
}

#[derive(Default)]
struct DumpCtx {
    next_temp: usize,
}

impl DumpCtx {
    fn dump_stmt(&mut self, stmt: &MirStmt, indent: usize, out: &mut String) {
        let pad = "  ".repeat(indent);
        match stmt {
            MirStmt::Local {
                mutable,
                name,
                ty,
                value,
            } => {
                let mutability = if *mutable { "mutable" } else { "immutable" };
                let ty = ty.as_deref().unwrap_or("<inferred>");
                out.push_str(&format!(
                    "{pad}local {name}: {ty} mutability={mutability}\n"
                ));
                let value_temp = self.dump_expr(value, indent, out);
                out.push_str(&format!("{pad}store {name}, {value_temp}\n"));
            }
            MirStmt::Store { name, value } => {
                let value_temp = self.dump_expr(value, indent, out);
                out.push_str(&format!("{pad}store {name}, {value_temp}\n"));
            }
            MirStmt::If {
                condition,
                then_body,
                else_body,
            } => {
                let condition_temp = self.dump_expr(condition, indent, out);
                out.push_str(&format!("{pad}if {condition_temp}\n"));
                out.push_str(&format!("{pad}  then\n"));
                for stmt in then_body {
                    self.dump_stmt(stmt, indent + 2, out);
                }
                if !else_body.is_empty() {
                    out.push_str(&format!("{pad}  else\n"));
                    for stmt in else_body {
                        self.dump_stmt(stmt, indent + 2, out);
                    }
                }
                out.push_str(&format!("{pad}end_if\n"));
            }
            MirStmt::While { condition, body } => {
                out.push_str(&format!("{pad}loop\n"));
                let condition_temp = self.dump_expr(condition, indent + 1, out);
                out.push_str(&format!("{pad}  break_unless {condition_temp}\n"));
                for stmt in body {
                    self.dump_stmt(stmt, indent + 1, out);
                }
                out.push_str(&format!("{pad}end_loop\n"));
            }
            MirStmt::Match { value, arms } => {
                let value_temp = self.dump_expr(value, indent, out);
                out.push_str(&format!("{pad}match {value_temp}\n"));
                for arm in arms {
                    out.push_str(&format!("{pad}  arm {}\n", pattern_text(&arm.pattern)));
                    for stmt in &arm.body {
                        self.dump_stmt(stmt, indent + 2, out);
                    }
                }
                out.push_str(&format!("{pad}end_match\n"));
            }
            MirStmt::Return(Some(value)) => {
                let value_temp = self.dump_expr(value, indent, out);
                out.push_str(&format!("{pad}return {value_temp}\n"));
            }
            MirStmt::Return(None) => {
                out.push_str(&format!("{pad}return Unit\n"));
            }
            MirStmt::Drop(value) => {
                let value_temp = self.dump_expr(value, indent, out);
                out.push_str(&format!("{pad}drop {value_temp}\n"));
            }
        }
    }

    fn dump_expr(&mut self, expr: &MirExpr, indent: usize, out: &mut String) -> String {
        let pad = "  ".repeat(indent);
        match expr {
            MirExpr::Load(name) => {
                let temp = self.temp();
                out.push_str(&format!("{pad}{temp} = load {name}\n"));
                temp
            }
            MirExpr::Int(value) => {
                let temp = self.temp();
                out.push_str(&format!("{pad}{temp} = const Int {value}\n"));
                temp
            }
            MirExpr::String(value) => {
                let temp = self.temp();
                out.push_str(&format!("{pad}{temp} = const String {value:?}\n"));
                temp
            }
            MirExpr::Bool(value) => {
                let temp = self.temp();
                out.push_str(&format!("{pad}{temp} = const Bool {value}\n"));
                temp
            }
            MirExpr::Binary { op, left, right } => {
                let left_temp = self.dump_expr(left, indent, out);
                let right_temp = self.dump_expr(right, indent, out);
                let temp = self.temp();
                out.push_str(&format!(
                    "{pad}{temp} = binary {} {left_temp}, {right_temp}\n",
                    binary_op_text(*op)
                ));
                temp
            }
            MirExpr::Unary { op, expr } => {
                let expr_temp = self.dump_expr(expr, indent, out);
                let temp = self.temp();
                out.push_str(&format!(
                    "{pad}{temp} = unary {} {expr_temp}\n",
                    unary_op_text(*op)
                ));
                temp
            }
            MirExpr::Call { callee, args } => {
                let mut arg_temps = Vec::new();
                for arg in args {
                    arg_temps.push(self.dump_expr(arg, indent, out));
                }
                let temp = self.temp();
                out.push_str(&format!(
                    "{pad}{temp} = call {callee}({})\n",
                    arg_temps.join(", ")
                ));
                temp
            }
            MirExpr::EnumVariant {
                enum_name,
                variant,
                payload,
            } => {
                let payload_temp = payload
                    .as_ref()
                    .map(|payload| self.dump_expr(payload, indent, out));
                let temp = self.temp();
                if let Some(payload_temp) = payload_temp {
                    out.push_str(&format!(
                        "{pad}{temp} = enum {enum_name}.{variant}({payload_temp})\n"
                    ));
                } else {
                    out.push_str(&format!("{pad}{temp} = enum {enum_name}.{variant}\n"));
                }
                temp
            }
            MirExpr::StructLiteral { ty, fields } => {
                let mut field_temps = Vec::new();
                for field in fields {
                    let value_temp = self.dump_expr(&field.value, indent, out);
                    field_temps.push(format!("{}: {}", field.name, value_temp));
                }
                let temp = self.temp();
                out.push_str(&format!(
                    "{pad}{temp} = struct {ty} {{ {} }}\n",
                    field_temps.join(", ")
                ));
                temp
            }
            MirExpr::FieldAccess { base, field } => {
                let base_temp = self.dump_expr(base, indent, out);
                let temp = self.temp();
                out.push_str(&format!("{pad}{temp} = field {base_temp}.{field}\n"));
                temp
            }
        }
    }

    fn temp(&mut self) -> String {
        let temp = format!("_t{}", self.next_temp);
        self.next_temp += 1;
        temp
    }
}

fn pattern_text(pattern: &MirPattern) -> String {
    match pattern {
        MirPattern::Name(name) => format!("name:{name}"),
        MirPattern::Variant {
            enum_name,
            variant,
            binding,
        } => {
            if let Some(binding) = binding {
                format!("variant:{enum_name}.{variant}({binding})")
            } else {
                format!("variant:{enum_name}.{variant}")
            }
        }
        MirPattern::Int(value) => format!("int:{value}"),
        MirPattern::String(value) => format!("string:{value:?}"),
        MirPattern::Bool(value) => format!("bool:{value}"),
        MirPattern::Wildcard => "_".to_string(),
    }
}

pub fn binary_op_text(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "add",
        BinaryOp::Sub => "sub",
        BinaryOp::Mul => "mul",
        BinaryOp::Div => "div",
        BinaryOp::And => "and",
        BinaryOp::Or => "or",
        BinaryOp::Eq => "eq",
        BinaryOp::NotEq => "not_eq",
        BinaryOp::Lt => "lt",
        BinaryOp::Lte => "lte",
        BinaryOp::Gt => "gt",
        BinaryOp::Gte => "gte",
    }
}

pub fn unary_op_text(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Not => "not",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MirType {
    Named(String),
    Unit,
    Unknown,
}

impl MirType {
    fn named(name: impl Into<String>) -> Self {
        Self::Named(name.into())
    }

    fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown)
    }

    fn display(&self) -> String {
        match self {
            Self::Named(name) => name.clone(),
            Self::Unit => "Unit".to_string(),
            Self::Unknown => "<unknown>".to_string(),
        }
    }
}

struct MirVerifier<'a> {
    program: &'a Program,
    functions: HashMap<&'a str, &'a MirFunction>,
    enums: HashMap<&'a str, HashMap<&'a str, Option<&'a str>>>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> MirVerifier<'a> {
    fn new(program: &'a Program) -> Self {
        Self {
            program,
            functions: program
                .functions
                .iter()
                .map(|function| (function.name.as_str(), function))
                .collect(),
            enums: program
                .enums
                .iter()
                .map(|enum_decl| {
                    (
                        enum_decl.name.as_str(),
                        enum_decl
                            .variants
                            .iter()
                            .map(|variant| (variant.name.as_str(), variant.payload_type.as_deref()))
                            .collect(),
                    )
                })
                .collect(),
            diagnostics: Vec::new(),
        }
    }

    fn verify_program(&mut self) {
        let mut names = HashSet::new();
        for function in &self.program.functions {
            if !names.insert(function.name.as_str()) {
                self.error(
                    "MIR_DUPLICATE_FUNCTION",
                    format!("duplicate MIR function `{}`", function.name),
                );
            }
            self.verify_function(function);
        }
    }

    fn verify_function(&mut self, function: &MirFunction) {
        let mut locals = HashMap::new();
        for param in &function.params {
            if locals
                .insert(param.name.clone(), MirType::named(param.ty.clone()))
                .is_some()
            {
                self.error(
                    "MIR_DUPLICATE_LOCAL",
                    format!("duplicate MIR parameter `{}`", param.name),
                );
            }
        }
        let expected_return = function
            .return_type
            .as_ref()
            .map(|ty| MirType::named(ty.clone()))
            .unwrap_or(MirType::Unit);
        let guarantees_return = self.verify_stmts(&function.body, &mut locals, &expected_return);
        if expected_return != MirType::Unit && !guarantees_return {
            self.error(
                "MIR_MISSING_RETURN",
                format!(
                    "function `{}` must return `{}` on all paths",
                    function.name,
                    expected_return.display()
                ),
            );
        }
    }

    fn verify_stmts(
        &mut self,
        stmts: &[MirStmt],
        locals: &mut HashMap<String, MirType>,
        expected_return: &MirType,
    ) -> bool {
        for stmt in stmts {
            if self.verify_stmt(stmt, locals, expected_return) {
                return true;
            }
        }
        false
    }

    fn verify_stmt(
        &mut self,
        stmt: &MirStmt,
        locals: &mut HashMap<String, MirType>,
        expected_return: &MirType,
    ) -> bool {
        match stmt {
            MirStmt::Local {
                name, ty, value, ..
            } => {
                if locals.contains_key(name) {
                    self.error(
                        "MIR_DUPLICATE_LOCAL",
                        format!("duplicate MIR local `{name}`"),
                    );
                }
                let value_ty = self.verify_expr(value, locals);
                let local_ty = ty
                    .as_ref()
                    .map(|ty| MirType::named(ty.clone()))
                    .unwrap_or_else(|| value_ty.clone());
                self.expect_type(
                    &value_ty,
                    &local_ty,
                    "MIR_LOCAL_TYPE",
                    format!(
                        "local `{name}` expects `{}`, found `{}`",
                        local_ty.display(),
                        value_ty.display()
                    ),
                );
                locals.insert(name.clone(), local_ty);
                false
            }
            MirStmt::Store { name, value } => {
                let value_ty = self.verify_expr(value, locals);
                let Some(local_ty) = locals.get(name).cloned() else {
                    self.error(
                        "MIR_UNKNOWN_LOCAL",
                        format!("store target `{name}` is not defined"),
                    );
                    return false;
                };
                self.expect_type(
                    &value_ty,
                    &local_ty,
                    "MIR_STORE_TYPE",
                    format!(
                        "store target `{name}` expects `{}`, found `{}`",
                        local_ty.display(),
                        value_ty.display()
                    ),
                );
                false
            }
            MirStmt::If {
                condition,
                then_body,
                else_body,
            } => {
                self.expect_bool(condition, locals, "MIR_IF_CONDITION", "if condition");
                let mut then_locals = locals.clone();
                let then_returns = self.verify_stmts(then_body, &mut then_locals, expected_return);
                let mut else_locals = locals.clone();
                let else_returns = self.verify_stmts(else_body, &mut else_locals, expected_return);
                then_returns && else_returns
            }
            MirStmt::While { condition, body } => {
                self.expect_bool(condition, locals, "MIR_WHILE_CONDITION", "while condition");
                let mut body_locals = locals.clone();
                self.verify_stmts(body, &mut body_locals, expected_return);
                false
            }
            MirStmt::Match { value, arms } => {
                let value_ty = self.verify_expr(value, locals);
                let mut all_arms_return = !arms.is_empty();
                let mut has_wildcard = false;
                for arm in arms {
                    if matches!(arm.pattern, MirPattern::Wildcard) {
                        has_wildcard = true;
                    }
                    let mut arm_locals = locals.clone();
                    for (name, ty) in self.verify_pattern(&arm.pattern, &value_ty) {
                        if arm_locals.insert(name.clone(), ty).is_some() {
                            self.error(
                                "MIR_DUPLICATE_LOCAL",
                                format!("duplicate MIR match binding `{name}`"),
                            );
                        }
                    }
                    all_arms_return &=
                        self.verify_stmts(&arm.body, &mut arm_locals, expected_return);
                }
                all_arms_return && has_wildcard
            }
            MirStmt::Return(Some(value)) => {
                let value_ty = self.verify_expr(value, locals);
                self.expect_type(
                    &value_ty,
                    expected_return,
                    "MIR_RETURN_TYPE",
                    format!(
                        "return expects `{}`, found `{}`",
                        expected_return.display(),
                        value_ty.display()
                    ),
                );
                true
            }
            MirStmt::Return(None) => {
                self.expect_type(
                    &MirType::Unit,
                    expected_return,
                    "MIR_RETURN_TYPE",
                    format!(
                        "return expects `{}`, found `Unit`",
                        expected_return.display()
                    ),
                );
                true
            }
            MirStmt::Drop(value) => {
                self.verify_expr(value, locals);
                false
            }
        }
    }

    fn verify_expr(&mut self, expr: &MirExpr, locals: &HashMap<String, MirType>) -> MirType {
        match expr {
            MirExpr::Load(name) => locals.get(name).cloned().unwrap_or_else(|| {
                self.error(
                    "MIR_UNKNOWN_LOCAL",
                    format!("load source `{name}` is not defined"),
                );
                MirType::Unknown
            }),
            MirExpr::Int(_) => MirType::named("Int"),
            MirExpr::String(_) => MirType::named("String"),
            MirExpr::Bool(_) => MirType::named("Bool"),
            MirExpr::Binary { op, left, right } => self.verify_binary(*op, left, right, locals),
            MirExpr::Unary { op, expr } => match op {
                UnaryOp::Not => {
                    self.expect_bool(expr, locals, "MIR_UNARY_TYPE", "not operand");
                    MirType::named("Bool")
                }
            },
            MirExpr::Call { callee, args } => self.verify_call(callee, args, locals),
            MirExpr::EnumVariant {
                enum_name,
                variant,
                payload,
            } => {
                let payload_type = self.verify_enum_variant(enum_name, variant);
                match (payload_type, payload) {
                    (Some(expected), Some(payload)) => {
                        let payload_ty = self.verify_expr(payload, locals);
                        self.expect_type(
                            &payload_ty,
                            &MirType::named(&expected),
                            "MIR_ENUM_PAYLOAD_TYPE",
                            format!(
                                "variant `{enum_name}.{variant}` expects payload `{expected}`, found `{}`",
                                payload_ty.display()
                            ),
                        );
                    }
                    (Some(_), None) => self.error(
                        "MIR_ENUM_PAYLOAD_ARITY",
                        format!("variant `{enum_name}.{variant}` requires a payload"),
                    ),
                    (None, Some(payload)) => {
                        self.verify_expr(payload, locals);
                        self.error(
                            "MIR_ENUM_PAYLOAD_ARITY",
                            format!("variant `{enum_name}.{variant}` does not accept a payload"),
                        );
                    }
                    (None, None) => {}
                }
                MirType::named(enum_name.clone())
            }
            MirExpr::StructLiteral { ty, fields } => {
                let mut seen = HashSet::new();
                for field in fields {
                    if !seen.insert(field.name.as_str()) {
                        self.error(
                            "MIR_DUPLICATE_FIELD",
                            format!("duplicate field `{}` in struct literal `{ty}`", field.name),
                        );
                    }
                    self.verify_expr(&field.value, locals);
                }
                MirType::named(ty.clone())
            }
            MirExpr::FieldAccess { base, .. } => {
                self.verify_expr(base, locals);
                MirType::Unknown
            }
        }
    }

    fn verify_binary(
        &mut self,
        op: BinaryOp,
        left: &MirExpr,
        right: &MirExpr,
        locals: &HashMap<String, MirType>,
    ) -> MirType {
        let left_ty = self.verify_expr(left, locals);
        let right_ty = self.verify_expr(right, locals);
        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                self.expect_named(&left_ty, "Int", "MIR_BINARY_TYPE", "left operand");
                self.expect_named(&right_ty, "Int", "MIR_BINARY_TYPE", "right operand");
                MirType::named("Int")
            }
            BinaryOp::And | BinaryOp::Or => {
                self.expect_named(&left_ty, "Bool", "MIR_BINARY_TYPE", "left operand");
                self.expect_named(&right_ty, "Bool", "MIR_BINARY_TYPE", "right operand");
                MirType::named("Bool")
            }
            BinaryOp::Lt | BinaryOp::Lte | BinaryOp::Gt | BinaryOp::Gte => {
                self.expect_named(&left_ty, "Int", "MIR_BINARY_TYPE", "left operand");
                self.expect_named(&right_ty, "Int", "MIR_BINARY_TYPE", "right operand");
                MirType::named("Bool")
            }
            BinaryOp::Eq | BinaryOp::NotEq => {
                self.expect_type(
                    &left_ty,
                    &right_ty,
                    "MIR_BINARY_TYPE",
                    format!(
                        "equality operands must match, found `{}` and `{}`",
                        left_ty.display(),
                        right_ty.display()
                    ),
                );
                MirType::named("Bool")
            }
        }
    }

    fn verify_call(
        &mut self,
        callee: &str,
        args: &[MirExpr],
        locals: &HashMap<String, MirType>,
    ) -> MirType {
        let Some(function) = self.functions.get(callee).copied() else {
            self.error(
                "MIR_UNKNOWN_FUNCTION",
                format!("call target `{callee}` is not defined"),
            );
            for arg in args {
                self.verify_expr(arg, locals);
            }
            return MirType::Unknown;
        };
        if function.params.len() != args.len() {
            self.error(
                "MIR_CALL_ARITY",
                format!(
                    "function `{callee}` expects {} arguments, found {}",
                    function.params.len(),
                    args.len()
                ),
            );
        }
        for (param, arg) in function.params.iter().zip(args) {
            let arg_ty = self.verify_expr(arg, locals);
            let param_ty = MirType::named(param.ty.clone());
            self.expect_type(
                &arg_ty,
                &param_ty,
                "MIR_CALL_TYPE",
                format!(
                    "argument `{}` for `{callee}` expects `{}`, found `{}`",
                    param.name,
                    param_ty.display(),
                    arg_ty.display()
                ),
            );
        }
        function
            .return_type
            .as_ref()
            .map(|ty| MirType::named(ty.clone()))
            .unwrap_or(MirType::Unit)
    }

    fn verify_pattern(
        &mut self,
        pattern: &MirPattern,
        value_ty: &MirType,
    ) -> Vec<(String, MirType)> {
        match pattern {
            MirPattern::Name(name) => return vec![(name.clone(), value_ty.clone())],
            MirPattern::Wildcard => {}
            MirPattern::Variant {
                enum_name,
                variant,
                binding,
            } => {
                let payload_type = self.verify_enum_variant(enum_name, variant);
                self.expect_type(
                    &MirType::named(enum_name.clone()),
                    value_ty,
                    "MIR_MATCH_PATTERN",
                    format!(
                        "match pattern `{enum_name}.{variant}` expects value type `{enum_name}`, found `{}`",
                        value_ty.display()
                    ),
                );
                match (payload_type, binding) {
                    (Some(payload_type), Some(binding)) => {
                        return vec![(binding.clone(), MirType::named(payload_type))];
                    }
                    (Some(_), None) => self.error(
                        "MIR_ENUM_PATTERN_ARITY",
                        format!("variant `{enum_name}.{variant}` requires a payload binding"),
                    ),
                    (None, Some(_)) => self.error(
                        "MIR_ENUM_PATTERN_ARITY",
                        format!("variant `{enum_name}.{variant}` does not carry a payload"),
                    ),
                    (None, None) => {}
                }
            }
            MirPattern::Int(_) => {
                self.expect_named(value_ty, "Int", "MIR_MATCH_PATTERN", "match pattern")
            }
            MirPattern::String(_) => {
                self.expect_named(value_ty, "String", "MIR_MATCH_PATTERN", "match pattern")
            }
            MirPattern::Bool(_) => {
                self.expect_named(value_ty, "Bool", "MIR_MATCH_PATTERN", "match pattern")
            }
        }
        Vec::new()
    }

    fn verify_enum_variant(&mut self, enum_name: &str, variant: &str) -> Option<String> {
        let Some(variants) = self.enums.get(enum_name) else {
            self.error(
                "MIR_UNKNOWN_ENUM",
                format!("enum `{enum_name}` is not defined"),
            );
            return None;
        };
        match variants.get(variant) {
            Some(payload_type) => payload_type.map(str::to_string),
            None => {
                self.error(
                    "MIR_UNKNOWN_VARIANT",
                    format!("enum `{enum_name}` has no variant `{variant}`"),
                );
                None
            }
        }
    }

    fn expect_bool(
        &mut self,
        expr: &MirExpr,
        locals: &HashMap<String, MirType>,
        code: &'static str,
        label: &str,
    ) {
        let ty = self.verify_expr(expr, locals);
        self.expect_named(&ty, "Bool", code, label);
    }

    fn expect_named(&mut self, actual: &MirType, expected: &str, code: &'static str, label: &str) {
        self.expect_type(
            actual,
            &MirType::named(expected),
            code,
            format!("{label} expects `{expected}`, found `{}`", actual.display()),
        );
    }

    fn expect_type(
        &mut self,
        actual: &MirType,
        expected: &MirType,
        code: &'static str,
        message: impl Into<String>,
    ) {
        if actual.is_unknown() || expected.is_unknown() || actual == expected {
            return;
        }
        self.error(code, message);
    }

    fn error(&mut self, code: &'static str, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::new(code, message, Span::new(0, 0)));
    }
}
