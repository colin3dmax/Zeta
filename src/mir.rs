use crate::ast::{BinaryOp, Expr, Function, Item, Module, Param, Stmt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub functions: Vec<MirFunction>,
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
    UnsupportedMatch,
    Return(Option<MirExpr>),
    Drop(MirExpr),
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
    Call {
        callee: String,
        args: Vec<MirExpr>,
    },
}

pub fn lower(module: &Module) -> Program {
    Program {
        functions: module
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Function(function) => Some(lower_function(function)),
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
    for function in &program.functions {
        dump_function(function, 1, &mut out);
    }
    out
}

fn lower_function(function: &Function) -> MirFunction {
    MirFunction {
        name: function.name.clone(),
        params: function.params.clone(),
        return_type: function.return_type.clone(),
        body: function.body.iter().map(lower_stmt).collect(),
    }
}

fn lower_stmt(stmt: &Stmt) -> MirStmt {
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
            value: lower_expr(value),
        },
        Stmt::Assign { name, value, .. } => MirStmt::Store {
            name: name.clone(),
            value: lower_expr(value),
        },
        Stmt::If {
            condition,
            then_body,
            else_body,
        } => MirStmt::If {
            condition: lower_expr(condition),
            then_body: then_body.iter().map(lower_stmt).collect(),
            else_body: else_body.iter().map(lower_stmt).collect(),
        },
        Stmt::While { condition, body } => MirStmt::While {
            condition: lower_expr(condition),
            body: body.iter().map(lower_stmt).collect(),
        },
        Stmt::Match { .. } => MirStmt::UnsupportedMatch,
        Stmt::Return(value) => MirStmt::Return(value.as_ref().map(lower_expr)),
        Stmt::Expr(value) => MirStmt::Drop(lower_expr(value)),
    }
}

fn lower_expr(expr: &Expr) -> MirExpr {
    match expr {
        Expr::Name { name, .. } => MirExpr::Load(name.clone()),
        Expr::Int { value, .. } => MirExpr::Int(value.clone()),
        Expr::String { value, .. } => MirExpr::String(value.clone()),
        Expr::Bool { value, .. } => MirExpr::Bool(*value),
        Expr::Binary {
            op, left, right, ..
        } => MirExpr::Binary {
            op: *op,
            left: Box::new(lower_expr(left)),
            right: Box::new(lower_expr(right)),
        },
        Expr::Call { callee, args, .. } => MirExpr::Call {
            callee: callee.clone(),
            args: args.iter().map(lower_expr).collect(),
        },
    }
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
            MirStmt::UnsupportedMatch => {
                out.push_str(&format!("{pad}unsupported match\n"));
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
        }
    }

    fn temp(&mut self) -> String {
        let temp = format!("_t{}", self.next_temp);
        self.next_temp += 1;
        temp
    }
}

pub fn binary_op_text(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "add",
        BinaryOp::Sub => "sub",
        BinaryOp::Mul => "mul",
        BinaryOp::Div => "div",
    }
}
