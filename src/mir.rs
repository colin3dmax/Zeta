use crate::ast::{BinaryOp, Expr, Function, Item, LambdaBody, Module, Param, Pattern, Stmt, UnaryOp};
use crate::diagnostic::{Diagnostic, Span};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub enums: Vec<MirEnum>,
    pub functions: Vec<MirFunction>,
    /// Names of trait methods (from `trait` items). A call to one of these
    /// dispatches by the first argument's concrete type to the flattened impl
    /// function `{method}${TargetBase}`; the verifier and interpreter consult
    /// this set to accept and route such calls.
    pub trait_methods: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirEnum {
    pub name: String,
    /// Generic type parameters (empty for non-generic). The verifier treats a
    /// `Named(P)` for any `P` here as a wildcard, so generic enum payload/match
    /// binding types check leniently (instantiation erased in this slice).
    pub type_params: Vec<String>,
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
    /// Generic type parameters (empty for non-generic). The verifier treats a
    /// `Named(P)` for any `P` here as compatible with everything; native
    /// monomorphization specializes these away before codegen.
    pub type_params: Vec<String>,
    pub params: Vec<Param>,
    pub return_type: Option<String>,
    pub body: Vec<MirStmt>,
    /// Carried from `reloadable fn` (see ast::Function::reloadable): marks this
    /// function as an opt-in hot-swap boundary for the runtime / native backend.
    pub reloadable: bool,
    /// Declared `extern fn` — no body; codegen emits only a declaration and the
    /// linker resolves the symbol. The interpreter cannot call it (native-only).
    pub is_extern: bool,
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
        place: MirPlace,
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
    ForIn {
        binding: String,
        iterable: MirExpr,
        body: Vec<MirStmt>,
    },
    ForRange {
        binding: String,
        start: MirExpr,
        end: MirExpr,
        body: Vec<MirStmt>,
    },
    ForC {
        init: Box<MirStmt>,
        condition: MirExpr,
        step: Box<MirStmt>,
        body: Vec<MirStmt>,
    },
    Match {
        value: MirExpr,
        arms: Vec<MirMatchArm>,
    },
    Return(Option<MirExpr>),
    Break,
    Continue,
    Drop(MirExpr),
}

/// 赋值左值(place):简单变量 `a`、字段 `a.b`、下标 `a[i]` 及其链式组合。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirPlace {
    Local(String),
    Field {
        base: Box<MirPlace>,
        field: String,
    },
    Index {
        base: Box<MirPlace>,
        index: Box<MirExpr>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirMatchArm {
    pub pattern: MirPattern,
    /// Optional guard expression (`pat if <cond> -> ..`); the arm is taken only
    /// when the pattern matches AND this evaluates true. `None` for a plain arm.
    pub guard: Option<MirExpr>,
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
    Float(String),
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
    ArrayLiteral {
        elements: Vec<MirExpr>,
    },
    Tuple {
        elements: Vec<MirExpr>,
    },
    Lambda {
        params: Vec<Param>,
        // Both expression and block lambda bodies collapse to a statement block
        // here (an expr body `|x| e` lowers to `[return e]`), so the lifted
        // function is emitted uniformly across backends.
        body: Vec<MirStmt>,
    },
    Index {
        base: Box<MirExpr>,
        index: Box<MirExpr>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirStructField {
    pub name: String,
    pub value: MirExpr,
}

pub fn lower(module: &Module) -> Program {
    lower_with_external_enum_variants(module, &HashMap::new(), &HashMap::new())
}

pub fn lower_with_external_enum_variants(
    module: &Module,
    external_enum_variants: &HashMap<String, HashMap<String, Option<String>>>,
    external_enum_type_params: &HashMap<String, Vec<String>>,
) -> Program {
    let mut enum_variants = enum_variants(module);
    for (enum_name, variants) in external_enum_variants {
        enum_variants.entry(enum_name.clone()).or_default().extend(
            variants
                .iter()
                .map(|(name, payload_type)| (name.clone(), payload_type.clone())),
        );
    }
    let mut enums = module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Enum(decl) => Some(MirEnum {
                name: decl.name.clone(),
                type_params: decl.type_params.clone(),
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
        .collect::<Vec<_>>();
    let local_enum_names = enums
        .iter()
        .map(|enum_decl| enum_decl.name.clone())
        .collect::<HashSet<_>>();
    let mut external_enums = external_enum_variants
        .iter()
        .filter(|(enum_name, _)| !local_enum_names.contains(*enum_name))
        .collect::<Vec<_>>();
    external_enums.sort_by_key(|(enum_name, _)| enum_name.as_str());
    enums.extend(external_enums.into_iter().map(|(enum_name, variants)| {
        let mut variants = variants
            .iter()
            .map(|(name, payload_type)| MirEnumVariant {
                name: name.clone(),
                payload_type: payload_type.clone(),
            })
            .collect::<Vec<_>>();
        variants.sort_by_key(|variant| variant.name.clone());
        MirEnum {
            name: enum_name.clone(),
            type_params: external_enum_type_params
                .get(enum_name)
                .cloned()
                .unwrap_or_default(),
            variants,
        }
    }));
    Program {
        enums,
        functions: module
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Function(function) => Some(lower_function(function, &enum_variants)),
                _ => None,
            })
            .collect(),
        trait_methods: module.trait_method_names(),
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
        type_params: function.type_params.clone(),
        params: function.params.clone(),
        return_type: function.return_type.clone(),
        body: function
            .body
            .iter()
            .map(|stmt| lower_stmt(stmt, enum_variants))
            .collect(),
        reloadable: function.reloadable,
        is_extern: function.is_extern,
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
        Stmt::Assign { target, value } => MirStmt::Store {
            place: lower_place(target, enum_variants),
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
        Stmt::ForIn {
            binding,
            iterable,
            body,
            ..
        } => {
            let lowered_body = body
                .iter()
                .map(|stmt| lower_stmt(stmt, enum_variants))
                .collect();
            if let Expr::Range { start, end, .. } = iterable {
                MirStmt::ForRange {
                    binding: binding.clone(),
                    start: lower_expr(start, enum_variants),
                    end: lower_expr(end, enum_variants),
                    body: lowered_body,
                }
            } else {
                MirStmt::ForIn {
                    binding: binding.clone(),
                    iterable: lower_expr(iterable, enum_variants),
                    body: lowered_body,
                }
            }
        }
        Stmt::ForC {
            init,
            condition,
            step,
            body,
        } => MirStmt::ForC {
            init: Box::new(lower_stmt(init, enum_variants)),
            condition: lower_expr(condition, enum_variants),
            step: Box::new(lower_stmt(step, enum_variants)),
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
                    guard: arm.guard.as_ref().map(|g| lower_expr(g, enum_variants)),
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
        Stmt::Break { .. } => MirStmt::Break,
        Stmt::Continue { .. } => MirStmt::Continue,
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

fn lower_place(
    target: &Expr,
    enum_variants: &HashMap<String, HashMap<String, Option<String>>>,
) -> MirPlace {
    match target {
        Expr::Name { name, .. } => MirPlace::Local(name.clone()),
        Expr::FieldAccess { base, field, .. } => MirPlace::Field {
            base: Box::new(lower_place(base, enum_variants)),
            field: field.clone(),
        },
        Expr::Index { base, index, .. } => MirPlace::Index {
            base: Box::new(lower_place(base, enum_variants)),
            index: Box::new(lower_expr(index, enum_variants)),
        },
        // typecheck 已保证 target 是合法 lvalue,理论上不会到这里。
        _ => MirPlace::Local(String::new()),
    }
}

fn lower_expr(
    expr: &Expr,
    enum_variants: &HashMap<String, HashMap<String, Option<String>>>,
) -> MirExpr {
    match expr {
        Expr::Try { .. } => unreachable!("`?` is desugared before MIR lowering"),
        Expr::Name { name, .. } => MirExpr::Load(name.clone()),
        Expr::Int { value, .. } => MirExpr::Int(value.clone()),
        Expr::Float { value, .. } => MirExpr::Float(value.clone()),
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
        Expr::ArrayLiteral { elements, .. } => MirExpr::ArrayLiteral {
            elements: elements
                .iter()
                .map(|element| lower_expr(element, enum_variants))
                .collect(),
        },
        Expr::Tuple { elements, .. } => MirExpr::Tuple {
            elements: elements
                .iter()
                .map(|element| lower_expr(element, enum_variants))
                .collect(),
        },
        Expr::Lambda { params, body, .. } => MirExpr::Lambda {
            params: params.clone(),
            body: match body {
                // An expression body becomes a single `return <expr>` so the lifted
                // function body is always a statement block.
                LambdaBody::Expr(e) => vec![MirStmt::Return(Some(lower_expr(e, enum_variants)))],
                LambdaBody::Block(stmts) => {
                    stmts.iter().map(|s| lower_stmt(s, enum_variants)).collect()
                }
            },
        },
        Expr::Index { base, index, .. } => MirExpr::Index {
            base: Box::new(lower_expr(base, enum_variants)),
            index: Box::new(lower_expr(index, enum_variants)),
        },
        // Range 只作为 for-in 的 iterable 出现,在 lower_stmt 里被拆成 ForRange 的 start/end,
        // 不会作为独立表达式 lower。
        Expr::Range { .. } => {
            unreachable!(
                "Expr::Range only appears as a for-in iterable and is lowered in lower_stmt"
            )
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
            MirStmt::Store { place, value } => {
                let value_temp = self.dump_expr(value, indent, out);
                let place_text = self.dump_place(place, indent, out);
                out.push_str(&format!("{pad}store {place_text}, {value_temp}\n"));
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
            MirStmt::ForIn {
                binding,
                iterable,
                body,
            } => {
                let iterable_temp = self.dump_expr(iterable, indent, out);
                out.push_str(&format!("{pad}for {binding} in {iterable_temp}\n"));
                for stmt in body {
                    self.dump_stmt(stmt, indent + 1, out);
                }
                out.push_str(&format!("{pad}end_for\n"));
            }
            MirStmt::ForRange {
                binding,
                start,
                end,
                body,
            } => {
                let start_temp = self.dump_expr(start, indent, out);
                let end_temp = self.dump_expr(end, indent, out);
                out.push_str(&format!("{pad}for {binding} in {start_temp}..{end_temp}\n"));
                for stmt in body {
                    self.dump_stmt(stmt, indent + 1, out);
                }
                out.push_str(&format!("{pad}end_for\n"));
            }
            MirStmt::ForC {
                init,
                condition,
                step,
                body,
            } => {
                out.push_str(&format!("{pad}for_c\n"));
                out.push_str(&format!("{pad}  init\n"));
                self.dump_stmt(init, indent + 2, out);
                let condition_temp = self.dump_expr(condition, indent + 1, out);
                out.push_str(&format!("{pad}  break_unless {condition_temp}\n"));
                out.push_str(&format!("{pad}  body\n"));
                for stmt in body {
                    self.dump_stmt(stmt, indent + 2, out);
                }
                out.push_str(&format!("{pad}  step\n"));
                self.dump_stmt(step, indent + 2, out);
                out.push_str(&format!("{pad}end_for_c\n"));
            }
            MirStmt::Match { value, arms } => {
                let value_temp = self.dump_expr(value, indent, out);
                out.push_str(&format!("{pad}match {value_temp}\n"));
                for arm in arms {
                    out.push_str(&format!("{pad}  arm {}\n", pattern_text(&arm.pattern)));
                    if let Some(guard) = &arm.guard {
                        let g = self.dump_expr(guard, indent + 2, out);
                        out.push_str(&format!("{pad}    guard {g}\n"));
                    }
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
            MirStmt::Break => {
                out.push_str(&format!("{pad}break\n"));
            }
            MirStmt::Continue => {
                out.push_str(&format!("{pad}continue\n"));
            }
            MirStmt::Drop(value) => {
                let value_temp = self.dump_expr(value, indent, out);
                out.push_str(&format!("{pad}drop {value_temp}\n"));
            }
        }
    }

    fn dump_place(&mut self, place: &MirPlace, indent: usize, out: &mut String) -> String {
        match place {
            MirPlace::Local(name) => name.clone(),
            MirPlace::Field { base, field } => {
                let base_text = self.dump_place(base, indent, out);
                format!("{base_text}.{field}")
            }
            MirPlace::Index { base, index } => {
                let base_text = self.dump_place(base, indent, out);
                let index_temp = self.dump_expr(index, indent, out);
                format!("{base_text}[{index_temp}]")
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
            MirExpr::Float(value) => {
                let temp = self.temp();
                out.push_str(&format!("{pad}{temp} = const Float {value}\n"));
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
            MirExpr::ArrayLiteral { elements } => {
                let mut element_temps = Vec::new();
                for element in elements {
                    element_temps.push(self.dump_expr(element, indent, out));
                }
                let temp = self.temp();
                out.push_str(&format!(
                    "{pad}{temp} = array [{}]\n",
                    element_temps.join(", ")
                ));
                temp
            }
            MirExpr::Tuple { elements } => {
                let mut element_temps = Vec::new();
                for element in elements {
                    element_temps.push(self.dump_expr(element, indent, out));
                }
                let temp = self.temp();
                out.push_str(&format!(
                    "{pad}{temp} = tuple ({})\n",
                    element_temps.join(", ")
                ));
                temp
            }
            MirExpr::Lambda { params, body } => {
                let names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
                let temp = self.temp();
                out.push_str(&format!("{pad}{temp} = lambda |{}|\n", names.join(", ")));
                // An expression-body lambda lowers to a single `return <expr>`; dump
                // it as just the expression (the historical format the self-hosting
                // frontend also emits) so stage-1 parity holds. Only genuine
                // multi-statement block bodies dump as statements.
                match body.as_slice() {
                    [MirStmt::Return(Some(expr))] => {
                        self.dump_expr(expr, indent + 1, out);
                    }
                    _ => {
                        for stmt in body {
                            self.dump_stmt(stmt, indent + 1, out);
                        }
                    }
                }
                temp
            }
            MirExpr::Index { base, index } => {
                let base_temp = self.dump_expr(base, indent, out);
                let index_temp = self.dump_expr(index, indent, out);
                let temp = self.temp();
                out.push_str(&format!("{pad}{temp} = index {base_temp}[{index_temp}]\n"));
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

/// The trait-method name a binary operator overloads to, for a NON-scalar
/// operand (`+` → `add`, `==` → `eq`, ...). Routes via UFCS/trait dispatch to
/// `{name}${TypeBase}`, exactly like a method call. `&&`/`||` are excluded —
/// they short-circuit and are boolean-only. Reuses [`binary_op_text`]'s names.
/// Whether a type name is a built-in scalar/aggregate (not an overloadable
/// user struct/enum) — used to gate operator-overload dispatch.
fn is_scalar_type_name(name: &str) -> bool {
    matches!(
        name,
        "Int" | "Float"
            | "Bool"
            | "String"
            | "Unit"
            | "IntArray"
            | "StringArray"
            | "BoolArray"
            | "FloatArray"
            | "Array"
            | "Tuple"
            | "Fn"
    )
}

pub fn operator_trait_method(op: BinaryOp) -> Option<&'static str> {
    match op {
        BinaryOp::And | BinaryOp::Or => None,
        other => Some(binary_op_text(other)),
    }
}

pub fn binary_op_text(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "add",
        BinaryOp::Sub => "sub",
        BinaryOp::Mul => "mul",
        BinaryOp::Div => "div",
        BinaryOp::Mod => "mod",
        BinaryOp::BitAnd => "bit_and",
        BinaryOp::BitOr => "bit_or",
        BinaryOp::BitXor => "bit_xor",
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
        UnaryOp::Neg => "neg",
        UnaryOp::BitNot => "bit_not",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MirType {
    Named(String),
    Array(Box<MirType>),
    Tuple(Vec<MirType>),
    Fn(Vec<MirType>, Box<MirType>),
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
            Self::Array(element) => match element.as_ref() {
                Self::Named(name) => format!("{name}Array"),
                other => format!("{}Array", other.display()),
            },
            Self::Tuple(elements) => {
                let inner: Vec<String> = elements.iter().map(|e| e.display()).collect();
                format!("({})", inner.join(", "))
            }
            Self::Fn(params, ret) => {
                let inner: Vec<String> = params.iter().map(|p| p.display()).collect();
                format!("fn({}) -> {}", inner.join(", "), ret.display())
            }
            Self::Unit => "Unit".to_string(),
            Self::Unknown => "<unknown>".to_string(),
        }
    }
}

fn parse_mir_type(name: &str) -> MirType {
    if let Some((params, ret)) = crate::type_syntax::fn_parts(name) {
        return MirType::Fn(
            params.iter().map(|p| parse_mir_type(p)).collect(),
            Box::new(parse_mir_type(ret)),
        );
    }
    if let Some(parts) = crate::type_syntax::tuple_parts(name) {
        return MirType::Tuple(parts.iter().map(|p| parse_mir_type(p)).collect());
    }
    // `Array<E>` is the generic array type (element may be a type parameter).
    if let Some((base, args)) = crate::type_syntax::generic_parts(name) {
        if base == "Array" && args.len() == 1 {
            return MirType::Array(Box::new(parse_mir_type(args[0])));
        }
        // Other generic instantiations `Box<Int>` are erased to the base name for
        // the verifier (checked leniently; native codegen reads the arguments
        // separately to monomorphize).
        return parse_mir_type(base);
    }
    match name {
        "IntArray" => MirType::Array(Box::new(MirType::named("Int"))),
        "StringArray" => MirType::Array(Box::new(MirType::named("String"))),
        "BoolArray" => MirType::Array(Box::new(MirType::named("Bool"))),
        "FloatArray" => MirType::Array(Box::new(MirType::named("Float"))),
        "Unit" => MirType::Unit,
        other => MirType::named(other),
    }
}

struct MirVerifier<'a> {
    program: &'a Program,
    functions: HashMap<&'a str, &'a MirFunction>,
    enums: HashMap<&'a str, HashMap<&'a str, Option<&'a str>>>,
    /// Union of every function's generic type parameters. A `Named(p)` for any
    /// `p` here is treated as a wildcard during type checking (generics are
    /// monomorphized away before native codegen).
    type_params: HashSet<String>,
    /// Trait method names — calls to these dispatch by receiver type at
    /// runtime/codegen, so the verifier checks their arguments leniently rather
    /// than flagging the unmangled name as unknown.
    trait_methods: HashSet<String>,
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
            type_params: program
                .functions
                .iter()
                .flat_map(|function| function.type_params.iter().cloned())
                .chain(
                    program
                        .enums
                        .iter()
                        .flat_map(|enum_decl| enum_decl.type_params.iter().cloned()),
                )
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
            trait_methods: program.trait_methods.iter().cloned().collect(),
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
        // Extern declarations have no body — nothing to verify (their signature
        // is checked at call sites; the symbol is resolved by the linker).
        if function.is_extern {
            return;
        }
        let mut locals = HashMap::new();
        for param in &function.params {
            if locals
                .insert(param.name.clone(), parse_mir_type(&param.ty))
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
            .map(|ty| parse_mir_type(ty))
            .unwrap_or(MirType::Unit);
        let guarantees_return = self.verify_stmts(&function.body, &mut locals, &expected_return, 0);
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
        loop_depth: usize,
    ) -> bool {
        for stmt in stmts {
            if self.verify_stmt(stmt, locals, expected_return, loop_depth) {
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
        loop_depth: usize,
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
                    .map(|ty| parse_mir_type(ty))
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
            MirStmt::Store { place, value } => {
                let value_ty = self.verify_expr(value, locals);
                let place_ty = self.verify_place(place, locals);
                self.expect_type(
                    &value_ty,
                    &place_ty,
                    "MIR_STORE_TYPE",
                    format!(
                        "store target expects `{}`, found `{}`",
                        place_ty.display(),
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
                let then_returns =
                    self.verify_stmts(then_body, &mut then_locals, expected_return, loop_depth);
                let mut else_locals = locals.clone();
                let else_returns =
                    self.verify_stmts(else_body, &mut else_locals, expected_return, loop_depth);
                then_returns && else_returns
            }
            MirStmt::While { condition, body } => {
                self.expect_bool(condition, locals, "MIR_WHILE_CONDITION", "while condition");
                let mut body_locals = locals.clone();
                self.verify_stmts(body, &mut body_locals, expected_return, loop_depth + 1);
                false
            }
            MirStmt::ForIn {
                binding,
                iterable,
                body,
            } => {
                let iterable_ty = self.verify_expr(iterable, locals);
                let element_ty = match iterable_ty {
                    MirType::Array(element_ty) => *element_ty,
                    MirType::Unknown => MirType::Unknown,
                    other => {
                        self.error(
                            "MIR_FOR_ITERABLE",
                            format!("for-in expects array, found `{}`", other.display()),
                        );
                        MirType::Unknown
                    }
                };
                let mut body_locals = locals.clone();
                body_locals.insert(binding.clone(), element_ty);
                self.verify_stmts(body, &mut body_locals, expected_return, loop_depth + 1);
                false
            }
            MirStmt::ForRange {
                binding,
                start,
                end,
                body,
            } => {
                let start_ty = self.verify_expr(start, locals);
                self.expect_named(&start_ty, "Int", "MIR_FOR_RANGE_BOUND", "range start");
                let end_ty = self.verify_expr(end, locals);
                self.expect_named(&end_ty, "Int", "MIR_FOR_RANGE_BOUND", "range end");
                let mut body_locals = locals.clone();
                body_locals.insert(binding.clone(), MirType::named("Int"));
                self.verify_stmts(body, &mut body_locals, expected_return, loop_depth + 1);
                false
            }
            MirStmt::ForC {
                init,
                condition,
                step,
                body,
            } => {
                let mut loop_locals = locals.clone();
                // init declares its binding in loop_locals (scoped to the for).
                self.verify_stmt(init, &mut loop_locals, expected_return, loop_depth);
                self.expect_bool(
                    condition,
                    &loop_locals,
                    "MIR_FORC_CONDITION",
                    "for condition",
                );
                self.verify_stmt(step, &mut loop_locals, expected_return, loop_depth + 1);
                self.verify_stmts(body, &mut loop_locals, expected_return, loop_depth + 1);
                false
            }
            MirStmt::Match { value, arms } => {
                let value_ty = self.verify_expr(value, locals);
                let mut all_arms_return = !arms.is_empty();
                let mut has_wildcard = false;
                let mut covered_enum_variants: HashMap<String, HashSet<String>> = HashMap::new();
                let mut covered_bool_patterns = HashSet::new();
                for arm in arms {
                    // A guarded arm can fail (the guard may be false), so it never
                    // contributes to exhaustiveness — even a `_ if c` isn't a
                    // catch-all. Only plain arms cover the scrutinee.
                    let guarded = arm.guard.is_some();
                    // A `Name` binding is a catch-all too (it matches any value),
                    // exactly like `_` — matching typecheck's exhaustiveness rule.
                    if !guarded && matches!(arm.pattern, MirPattern::Wildcard | MirPattern::Name(_))
                    {
                        has_wildcard = true;
                    }
                    if !guarded {
                        if let MirPattern::Bool(value) = &arm.pattern {
                            covered_bool_patterns.insert(*value);
                        }
                        if let MirPattern::Variant {
                            enum_name, variant, ..
                        } = &arm.pattern
                        {
                            covered_enum_variants
                                .entry(enum_name.clone())
                                .or_default()
                                .insert(variant.clone());
                        }
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
                    // The guard is a Bool, evaluated with the pattern bindings in
                    // scope. A guarded arm's body need not return for exhaustiveness
                    // (control may fall through), so don't fold it into all_arms_return.
                    if let Some(guard) = &arm.guard {
                        let guard_ty = self.verify_expr(guard, &arm_locals);
                        self.expect_type(
                            &guard_ty,
                            &MirType::named("Bool"),
                            "MIR_MATCH_GUARD_NOT_BOOL",
                            "match guard must be a Bool",
                        );
                    }
                    let arm_returns =
                        self.verify_stmts(&arm.body, &mut arm_locals, expected_return, loop_depth);
                    if !guarded {
                        all_arms_return &= arm_returns;
                    }
                }
                all_arms_return
                    && (has_wildcard
                        || self.bool_match_is_exhaustive(&value_ty, &covered_bool_patterns)
                        || self.enum_match_is_exhaustive(&value_ty, &covered_enum_variants))
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
            MirStmt::Break => {
                if loop_depth == 0 {
                    self.error("MIR_BREAK_OUTSIDE_LOOP", "`break` appears outside a loop");
                }
                false
            }
            MirStmt::Continue => {
                if loop_depth == 0 {
                    self.error(
                        "MIR_CONTINUE_OUTSIDE_LOOP",
                        "`continue` appears outside a loop",
                    );
                }
                false
            }
            MirStmt::Drop(value) => {
                self.verify_expr(value, locals);
                false
            }
        }
    }

    fn verify_place(&mut self, place: &MirPlace, locals: &HashMap<String, MirType>) -> MirType {
        match place {
            MirPlace::Local(name) => match locals.get(name).cloned() {
                Some(ty) => ty,
                None => {
                    self.error(
                        "MIR_UNKNOWN_LOCAL",
                        format!("store target `{name}` is not defined"),
                    );
                    MirType::Unknown
                }
            },
            MirPlace::Field { base, .. } => {
                self.verify_place(base, locals);
                MirType::Unknown
            }
            MirPlace::Index { base, index } => {
                self.expect_int_index(index, locals);
                self.verify_place(base, locals);
                MirType::Unknown
            }
        }
    }

    /// Infer a lambda block's return type for the verifier: the type of the first
    /// `return <expr>` reached, with `let` bindings accumulated into scope.
    /// Recurses into nested control flow (each nested block scopes its own lets).
    fn block_return_mir_type(
        &mut self,
        stmts: &[MirStmt],
        scope: &mut HashMap<String, MirType>,
    ) -> MirType {
        for stmt in stmts {
            match stmt {
                MirStmt::Local { name, value, .. } => {
                    let t = self.verify_expr(value, scope);
                    scope.insert(name.clone(), t);
                }
                MirStmt::Return(Some(e)) => return self.verify_expr(e, scope),
                MirStmt::Return(None) => return MirType::named("Unit"),
                MirStmt::If { then_body, else_body, .. } => {
                    let mut s = scope.clone();
                    let t = self.block_return_mir_type(then_body, &mut s);
                    if !t.is_unknown() {
                        return t;
                    }
                    let mut s = scope.clone();
                    let t = self.block_return_mir_type(else_body, &mut s);
                    if !t.is_unknown() {
                        return t;
                    }
                }
                MirStmt::While { body, .. }
                | MirStmt::ForIn { body, .. }
                | MirStmt::ForRange { body, .. }
                | MirStmt::ForC { body, .. } => {
                    let mut s = scope.clone();
                    let t = self.block_return_mir_type(body, &mut s);
                    if !t.is_unknown() {
                        return t;
                    }
                }
                MirStmt::Match { arms, .. } => {
                    for arm in arms {
                        let mut s = scope.clone();
                        let t = self.block_return_mir_type(&arm.body, &mut s);
                        if !t.is_unknown() {
                            return t;
                        }
                    }
                }
                _ => {}
            }
        }
        MirType::Unknown
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
            MirExpr::Float(_) => MirType::named("Float"),
            MirExpr::String(_) => MirType::named("String"),
            MirExpr::Bool(_) => MirType::named("Bool"),
            MirExpr::Binary { op, left, right } => self.verify_binary(*op, left, right, locals),
            MirExpr::Unary { op, expr } => match op {
                UnaryOp::Not => {
                    self.expect_bool(expr, locals, "MIR_UNARY_TYPE", "not operand");
                    MirType::named("Bool")
                }
                UnaryOp::Neg => {
                    let ty = self.verify_expr(expr, locals);
                    let numeric = if matches!(&ty, MirType::Named(n) if n == "Float") {
                        "Float"
                    } else {
                        "Int"
                    };
                    self.expect_named(&ty, numeric, "MIR_UNARY_TYPE", "neg operand");
                    MirType::named(numeric)
                }
                UnaryOp::BitNot => {
                    let ty = self.verify_expr(expr, locals);
                    self.expect_named(&ty, "Int", "MIR_UNARY_TYPE", "bit_not operand");
                    MirType::named("Int")
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
                            &parse_mir_type(&expected),
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
            MirExpr::FieldAccess { base, field } => {
                let base_ty = self.verify_expr(base, locals);
                if matches!(base_ty, MirType::Array(_)) && field == "len" {
                    return MirType::named("Int");
                }
                if let MirType::Tuple(elements) = &base_ty {
                    match field.parse::<usize>() {
                        Ok(index) if index < elements.len() => return elements[index].clone(),
                        _ => {
                            self.error(
                                "MIR_TUPLE_INDEX",
                                format!(
                                    "tuple index `.{field}` out of range for {}-element tuple",
                                    elements.len()
                                ),
                            );
                            return MirType::Unknown;
                        }
                    }
                }
                MirType::Unknown
            }
            MirExpr::Tuple { elements } => {
                let types: Vec<MirType> = elements
                    .iter()
                    .map(|element| self.verify_expr(element, locals))
                    .collect();
                MirType::Tuple(types)
            }
            MirExpr::Lambda { params, body } => {
                // Verify the block body with the lambda's params added to scope.
                let mut body_locals = locals.clone();
                let mut param_types = Vec::with_capacity(params.len());
                for param in params {
                    let ty = parse_mir_type(&param.ty);
                    body_locals.insert(param.name.clone(), ty.clone());
                    param_types.push(ty);
                }
                // The return type is inferred from the block's `return` statements;
                // then the body is verified against it.
                let ret = self.block_return_mir_type(body, &mut body_locals.clone());
                self.verify_stmts(body, &mut body_locals, &ret, 0);
                MirType::Fn(param_types, Box::new(ret))
            }
            MirExpr::ArrayLiteral { elements } => {
                let Some((first, rest)) = elements.split_first() else {
                    self.error("MIR_ARRAY_EMPTY", "empty MIR arrays need an element type");
                    return MirType::Unknown;
                };
                let element_ty = self.verify_expr(first, locals);
                for element in rest {
                    let found = self.verify_expr(element, locals);
                    self.expect_type(
                        &found,
                        &element_ty,
                        "MIR_ARRAY_ELEMENT_TYPE",
                        format!(
                            "array element expects `{}`, found `{}`",
                            element_ty.display(),
                            found.display()
                        ),
                    );
                }
                MirType::Array(Box::new(element_ty))
            }
            MirExpr::Index { base, index } => {
                let base_ty = self.verify_expr(base, locals);
                self.expect_int_index(index, locals);
                match base_ty {
                    MirType::Array(element_ty) => *element_ty,
                    MirType::Unknown => MirType::Unknown,
                    other => {
                        self.error(
                            "MIR_INDEX_BASE",
                            format!("index base expects array, found `{}`", other.display()),
                        );
                        MirType::Unknown
                    }
                }
            }
        }
    }

    fn expect_int_index(&mut self, expr: &MirExpr, locals: &HashMap<String, MirType>) {
        let ty = self.verify_expr(expr, locals);
        self.expect_named(&ty, "Int", "MIR_INDEX_TYPE", "index");
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
        // Operator overloading: a non-scalar (struct/enum) left operand dispatches
        // to the operator's trait method (native/interpreter). Lenient here — the
        // result type is established by typecheck; the verifier just returns the
        // method's declared return (or Unknown).
        if let MirType::Named(base) = &left_ty {
            if !is_scalar_type_name(base) && operator_trait_method(op).is_some() {
                let dispatch = crate::type_syntax::dispatch_name(
                    operator_trait_method(op).unwrap(),
                    base,
                );
                return self
                    .functions
                    .get(dispatch.as_str())
                    .and_then(|f| f.return_type.as_deref())
                    .map(parse_mir_type)
                    .unwrap_or(MirType::Unknown);
            }
        }
        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                // Numeric: Int or Float (operands must match the left's type).
                let numeric = if matches!(&left_ty, MirType::Named(n) if n == "Float") {
                    "Float"
                } else {
                    "Int"
                };
                self.expect_named(&left_ty, numeric, "MIR_BINARY_TYPE", "left operand");
                self.expect_named(&right_ty, numeric, "MIR_BINARY_TYPE", "right operand");
                MirType::named(numeric)
            }
            BinaryOp::Mod | BinaryOp::BitAnd | BinaryOp::BitOr | BinaryOp::BitXor => {
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
                let numeric = if matches!(&left_ty, MirType::Named(n) if n == "Float") {
                    "Float"
                } else {
                    "Int"
                };
                self.expect_named(&left_ty, numeric, "MIR_BINARY_TYPE", "left operand");
                self.expect_named(&right_ty, numeric, "MIR_BINARY_TYPE", "right operand");
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
        if let Some(return_type) = self.verify_std_call(callee, args, locals) {
            return return_type;
        }
        // Generic array intrinsics: element type flows from an argument.
        if callee == "array_push" {
            // array_push(arr, x) -> arr's (array) type.
            let arr_ty = args
                .first()
                .map(|a| self.verify_expr(a, locals))
                .unwrap_or(MirType::Unknown);
            for arg in args.iter().skip(1) {
                self.verify_expr(arg, locals);
            }
            return arr_ty;
        }
        if callee == "array_repeat" {
            // array_repeat(value, count) -> Array<value type>.
            let elem = args
                .first()
                .map(|v| self.verify_expr(v, locals))
                .unwrap_or(MirType::Unknown);
            for arg in args.iter().skip(1) {
                self.verify_expr(arg, locals);
            }
            return MirType::Array(Box::new(elem));
        }
        // Raw-pointer intrinsics (unsafe, native-only). Checked leniently: the
        // verifier just walks the arguments. `ptr_addr` yields Int; the others'
        // element type is recovered by native codegen, so Unknown here.
        if matches!(
            callee,
            "ptr_from_addr" | "ptr_addr" | "ptr_read" | "ptr_write" | "ptr_offset"
        ) {
            for arg in args {
                self.verify_expr(arg, locals);
            }
            return if callee == "ptr_addr" || callee == "ptr_write" {
                MirType::named("Int")
            } else {
                MirType::Unknown
            };
        }
        if callee == "array_data_addr" {
            for arg in args {
                self.verify_expr(arg, locals);
            }
            return MirType::named("Int");
        }
        let Some(function) = self.functions.get(callee).copied() else {
            // Indirect call through a local of function type.
            if let Some(MirType::Fn(params, ret)) = locals.get(callee).cloned() {
                if params.len() != args.len() {
                    self.error(
                        "MIR_CALL_ARITY",
                        format!(
                            "closure `{callee}` expects {} arguments, found {}",
                            params.len(),
                            args.len()
                        ),
                    );
                }
                for (param_ty, arg) in params.iter().zip(args) {
                    let arg_ty = self.verify_expr(arg, locals);
                    self.expect_type(
                        &arg_ty,
                        param_ty,
                        "MIR_CALL_TYPE",
                        format!(
                            "argument for `{callee}` expects `{}`, found `{}`",
                            param_ty.display(),
                            arg_ty.display()
                        ),
                    );
                }
                return *ret;
            }
            // A trait method call (`show(p)`) dispatches by receiver type to a
            // flattened impl (`show$Point`) at runtime/codegen; check the
            // arguments but treat the result leniently (Unknown is a wildcard).
            if self.trait_methods.contains(callee) {
                for arg in args {
                    self.verify_expr(arg, locals);
                }
                return MirType::Unknown;
            }
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
            let param_ty = parse_mir_type(&param.ty);
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
            .map(|ty| parse_mir_type(ty))
            .unwrap_or(MirType::Unit)
    }

    fn verify_std_call(
        &mut self,
        callee: &str,
        args: &[MirExpr],
        locals: &HashMap<String, MirType>,
    ) -> Option<MirType> {
        let (params, return_ty) = match callee {
            "string_len" => (&["String"][..], MirType::named("Int")),
            "string_byte_at" => (&["String", "Int"][..], MirType::named("Int")),
            "string_byte_slice" => (&["String", "Int", "Int"][..], MirType::named("String")),
            "string_concat" => (&["String", "String"][..], MirType::named("String")),
            "int_to_string" => (&["Int"][..], MirType::named("String")),
            "int_abs" => (&["Int"][..], MirType::named("Int")),
            "int_min" | "int_max" | "int_pow" => (&["Int", "Int"][..], MirType::named("Int")),
            "string_to_int" => (&["String"][..], MirType::named("Int")),
            "string_index_of" => (&["String", "String"][..], MirType::named("Int")),
            "string_contains" => (&["String", "String"][..], MirType::named("Bool")),
            "string_repeat" => (&["String", "Int"][..], MirType::named("String")),
            "string_to_upper" | "string_to_lower" | "string_trim" => {
                (&["String"][..], MirType::named("String"))
            }
            "mmio_write_byte" | "mmio_write_word" | "mmio_write_dword" => {
                (&["Int", "Int"][..], MirType::named("Int"))
            }
            "mmio_read_byte" | "mmio_read_word" | "mmio_read_dword" => {
                (&["Int"][..], MirType::named("Int"))
            }
            "csr_read" => (&["Int"][..], MirType::named("Int")),
            "csr_write" | "csr_set" | "csr_clear" => {
                (&["Int", "Int"][..], MirType::named("Int"))
            }
            "wfi" => (&[][..], MirType::named("Int")),
            "ascii_is_digit" | "ascii_is_alpha" | "ascii_is_alnum" | "ascii_is_whitespace" => {
                (&["Int"][..], MirType::named("Bool"))
            }
            "int_array_empty" => (&[][..], parse_mir_type("IntArray")),
            "int_array_push" => (&["IntArray", "Int"][..], parse_mir_type("IntArray")),
            "string_array_empty" => (&[][..], parse_mir_type("StringArray")),
            "string_array_push" => (
                &["StringArray", "String"][..],
                parse_mir_type("StringArray"),
            ),
            "bool_array_empty" => (&[][..], parse_mir_type("BoolArray")),
            "bool_array_push" => (&["BoolArray", "Bool"][..], parse_mir_type("BoolArray")),
            "float_array_empty" => (&[][..], parse_mir_type("FloatArray")),
            "float_array_push" => (&["FloatArray", "Float"][..], parse_mir_type("FloatArray")),
            "print" | "println" => (&["String"][..], MirType::named("Int")),
            "file_read_to_string" => (&["String"][..], parse_mir_type("ResultString")),
            "path_join" => (&["String", "String"][..], parse_mir_type("String")),
            "path_basename" => (&["String"][..], parse_mir_type("String")),
            "diagnostic_format" => (
                &["String", "Int", "Int", "String"][..],
                parse_mir_type("String"),
            ),
            _ => return None,
        };
        if args.len() != params.len() {
            self.error(
                "MIR_CALL_ARITY",
                format!(
                    "function `{callee}` expects {} arguments, found {}",
                    params.len(),
                    args.len()
                ),
            );
        }
        for (index, (arg, expected)) in args.iter().zip(params.iter()).enumerate() {
            let arg_ty = self.verify_expr(arg, locals);
            let param_ty = parse_mir_type(expected);
            self.expect_type(
                &arg_ty,
                &param_ty,
                "MIR_CALL_TYPE",
                format!(
                    "argument {} for `{callee}` expects `{}`, found `{}`",
                    index + 1,
                    param_ty.display(),
                    arg_ty.display()
                ),
            );
        }
        Some(return_ty)
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
                        return vec![(binding.clone(), parse_mir_type(&payload_type))];
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

    fn enum_match_is_exhaustive(
        &self,
        value_ty: &MirType,
        covered: &HashMap<String, HashSet<String>>,
    ) -> bool {
        let MirType::Named(enum_name) = value_ty else {
            return false;
        };
        let Some(variants) = self.enums.get(enum_name.as_str()) else {
            return false;
        };
        let Some(covered_variants) = covered.get(enum_name) else {
            return false;
        };
        variants
            .keys()
            .all(|variant| covered_variants.contains(*variant))
    }

    fn bool_match_is_exhaustive(&self, value_ty: &MirType, covered: &HashSet<bool>) -> bool {
        matches!(value_ty, MirType::Named(name) if name == "Bool")
            && covered.contains(&true)
            && covered.contains(&false)
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
        if self.compatible(actual, expected) {
            return;
        }
        self.error(code, message);
    }

    /// Structural type compatibility where `Unknown` and any generic type
    /// parameter act as wildcards (matching anything), recursing through
    /// array/tuple/function types.
    fn compatible(&self, a: &MirType, b: &MirType) -> bool {
        if a.is_unknown() || b.is_unknown() {
            return true;
        }
        if let MirType::Named(n) = a {
            if self.type_params.contains(n) {
                return true;
            }
        }
        if let MirType::Named(n) = b {
            if self.type_params.contains(n) {
                return true;
            }
        }
        match (a, b) {
            (MirType::Named(x), MirType::Named(y)) => x == y,
            (MirType::Array(x), MirType::Array(y)) => self.compatible(x, y),
            (MirType::Tuple(xs), MirType::Tuple(ys)) => {
                xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| self.compatible(x, y))
            }
            (MirType::Fn(xp, xr), MirType::Fn(yp, yr)) => {
                xp.len() == yp.len()
                    && xp.iter().zip(yp).all(|(x, y)| self.compatible(x, y))
                    && self.compatible(xr, yr)
            }
            (MirType::Unit, MirType::Unit) => true,
            _ => false,
        }
    }

    fn error(&mut self, code: &'static str, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::new(code, message, Span::new(0, 0)));
    }
}
