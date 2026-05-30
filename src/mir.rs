use crate::ast::{BinaryOp, Expr, Function, Item, Module, Param, Pattern, Stmt, UnaryOp};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub enums: Vec<MirEnum>,
    pub functions: Vec<MirFunction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirEnum {
    pub name: String,
    pub variants: Vec<String>,
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
    Variant { enum_name: String, variant: String },
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
                    variants: decl.variants.clone(),
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
            enum_decl.variants.join(",")
        ));
    }
    for function in &program.functions {
        dump_function(function, 1, &mut out);
    }
    out
}

fn lower_function(
    function: &Function,
    enum_variants: &HashMap<String, Vec<String>>,
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

fn lower_stmt(stmt: &Stmt, enum_variants: &HashMap<String, Vec<String>>) -> MirStmt {
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
        Pattern::Variant { enum_name, variant } => MirPattern::Variant {
            enum_name: enum_name.clone(),
            variant: variant.clone(),
        },
        Pattern::Int(value) => MirPattern::Int(value.clone()),
        Pattern::String(value) => MirPattern::String(value.clone()),
        Pattern::Bool(value) => MirPattern::Bool(*value),
        Pattern::Wildcard => MirPattern::Wildcard,
    }
}

fn lower_expr(expr: &Expr, enum_variants: &HashMap<String, Vec<String>>) -> MirExpr {
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
        Expr::Call { callee, args, .. } => MirExpr::Call {
            callee: callee.clone(),
            args: args
                .iter()
                .map(|arg| lower_expr(arg, enum_variants))
                .collect(),
        },
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
                    .is_some_and(|variants| variants.iter().any(|variant| variant == field))
                {
                    return MirExpr::EnumVariant {
                        enum_name: enum_name.clone(),
                        variant: field.clone(),
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

fn enum_variants(module: &Module) -> HashMap<String, Vec<String>> {
    module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Enum(decl) => Some((decl.name.clone(), decl.variants.clone())),
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
            MirExpr::EnumVariant { enum_name, variant } => {
                let temp = self.temp();
                out.push_str(&format!("{pad}{temp} = enum {enum_name}.{variant}\n"));
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
        MirPattern::Variant { enum_name, variant } => format!("variant:{enum_name}.{variant}"),
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
