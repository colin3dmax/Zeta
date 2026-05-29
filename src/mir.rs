use crate::ast::{BinaryOp, Expr, Function, Item, Module, Stmt};

pub fn dump(module: &Module) -> String {
    let mut out = String::from("MirModule\n");
    for item in &module.items {
        if let Item::Function(function) = item {
            dump_function(function, 1, &mut out);
        }
    }
    out
}

fn dump_function(function: &Function, indent: usize, out: &mut String) {
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
    fn dump_stmt(&mut self, stmt: &Stmt, indent: usize, out: &mut String) {
        let pad = "  ".repeat(indent);
        match stmt {
            Stmt::Let {
                mutable,
                name,
                ty,
                value,
                ..
            } => {
                let mutability = if *mutable { "mutable" } else { "immutable" };
                let ty = ty.as_deref().unwrap_or("<inferred>");
                out.push_str(&format!(
                    "{pad}local {name}: {ty} mutability={mutability}\n"
                ));
                let value_temp = self.dump_expr(value, indent, out);
                out.push_str(&format!("{pad}store {name}, {value_temp}\n"));
            }
            Stmt::Assign { name, value, .. } => {
                let value_temp = self.dump_expr(value, indent, out);
                out.push_str(&format!("{pad}store {name}, {value_temp}\n"));
            }
            Stmt::If {
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
            Stmt::While { condition, body } => {
                out.push_str(&format!("{pad}loop\n"));
                let condition_temp = self.dump_expr(condition, indent + 1, out);
                out.push_str(&format!("{pad}  break_unless {condition_temp}\n"));
                for stmt in body {
                    self.dump_stmt(stmt, indent + 1, out);
                }
                out.push_str(&format!("{pad}end_loop\n"));
            }
            Stmt::Match { .. } => {
                out.push_str(&format!("{pad}unsupported match\n"));
            }
            Stmt::Return(Some(value)) => {
                let value_temp = self.dump_expr(value, indent, out);
                out.push_str(&format!("{pad}return {value_temp}\n"));
            }
            Stmt::Return(None) => {
                out.push_str(&format!("{pad}return Unit\n"));
            }
            Stmt::Expr(value) => {
                let value_temp = self.dump_expr(value, indent, out);
                out.push_str(&format!("{pad}drop {value_temp}\n"));
            }
        }
    }

    fn dump_expr(&mut self, expr: &Expr, indent: usize, out: &mut String) -> String {
        let pad = "  ".repeat(indent);
        match expr {
            Expr::Name { name, .. } => {
                let temp = self.temp();
                out.push_str(&format!("{pad}{temp} = load {name}\n"));
                temp
            }
            Expr::Int { value, .. } => {
                let temp = self.temp();
                out.push_str(&format!("{pad}{temp} = const Int {value}\n"));
                temp
            }
            Expr::String { value, .. } => {
                let temp = self.temp();
                out.push_str(&format!("{pad}{temp} = const String {value:?}\n"));
                temp
            }
            Expr::Bool { value, .. } => {
                let temp = self.temp();
                out.push_str(&format!("{pad}{temp} = const Bool {value}\n"));
                temp
            }
            Expr::Binary {
                op, left, right, ..
            } => {
                let left_temp = self.dump_expr(left, indent, out);
                let right_temp = self.dump_expr(right, indent, out);
                let temp = self.temp();
                out.push_str(&format!(
                    "{pad}{temp} = binary {} {left_temp}, {right_temp}\n",
                    binary_op_text(*op)
                ));
                temp
            }
            Expr::Call { callee, args, .. } => {
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

fn binary_op_text(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "add",
        BinaryOp::Sub => "sub",
        BinaryOp::Mul => "mul",
        BinaryOp::Div => "div",
    }
}
