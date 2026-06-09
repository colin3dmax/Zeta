use crate::ast::{BinaryOp, Expr, Item, Module, Pattern, Stmt, UnaryOp};

pub fn dump(module: &Module) -> String {
    let mut out = String::from("HirModule\n");
    for item in &module.items {
        dump_item(item, 1, &mut out);
    }
    out
}

fn dump_item(item: &Item, indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    match item {
        Item::ModuleDecl { name, .. } => {
            out.push_str(&format!("{pad}module {name}\n"));
        }
        Item::Import {
            exported,
            path,
            alias,
            ..
        } => {
            let visibility = if *exported { " exported" } else { "" };
            if let Some(alias) = alias {
                out.push_str(&format!(
                    "{pad}import{} {} as {alias}\n",
                    visibility,
                    path.join(".")
                ));
            } else {
                out.push_str(&format!("{pad}import{} {}\n", visibility, path.join(".")));
            }
        }
        Item::Struct(decl) => {
            out.push_str(&format!(
                "{pad}type struct {} visibility={}\n",
                decl.name,
                visibility(decl.exported)
            ));
            for field in &decl.fields {
                out.push_str(&format!("{pad}  field {}: {}\n", field.name, field.ty));
            }
        }
        Item::Enum(decl) => {
            out.push_str(&format!(
                "{pad}type enum {} visibility={}\n",
                decl.name,
                visibility(decl.exported)
            ));
            for variant in &decl.variants {
                out.push_str(&format!("{pad}  variant {}", variant.name));
                if let Some(payload_type) = &variant.payload_type {
                    out.push_str(&format!(" payload={payload_type}"));
                }
                out.push('\n');
            }
        }
        Item::Function(function) => {
            let return_type = function.return_type.as_deref().unwrap_or("Unit");
            out.push_str(&format!(
                "{pad}fn {} visibility={} -> {return_type}\n",
                function.name,
                visibility(function.exported)
            ));
            for param in &function.params {
                out.push_str(&format!("{pad}  param {}: {}\n", param.name, param.ty));
            }
            out.push_str(&format!("{pad}  body\n"));
            for stmt in &function.body {
                dump_stmt(stmt, indent + 2, out);
            }
        }
    }
}

fn dump_stmt(stmt: &Stmt, indent: usize, out: &mut String) {
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
            out.push_str(&format!("{pad}let {name}: {ty} mutability={mutability}\n"));
            dump_expr(value, indent + 1, out);
        }
        Stmt::Assign { target, value } => match target {
            Expr::Name { name, .. } => {
                out.push_str(&format!("{pad}assign {name}\n"));
                dump_expr(value, indent + 1, out);
            }
            _ => {
                out.push_str(&format!("{pad}assign\n"));
                dump_expr(target, indent + 1, out);
                dump_expr(value, indent + 1, out);
            }
        },
        Stmt::If {
            condition,
            then_body,
            else_body,
        } => {
            out.push_str(&format!("{pad}if\n"));
            out.push_str(&format!("{pad}  condition\n"));
            dump_expr(condition, indent + 2, out);
            out.push_str(&format!("{pad}  then\n"));
            for stmt in then_body {
                dump_stmt(stmt, indent + 2, out);
            }
            if !else_body.is_empty() {
                out.push_str(&format!("{pad}  else\n"));
                for stmt in else_body {
                    dump_stmt(stmt, indent + 2, out);
                }
            }
        }
        Stmt::While { condition, body } => {
            out.push_str(&format!("{pad}while\n"));
            out.push_str(&format!("{pad}  condition\n"));
            dump_expr(condition, indent + 2, out);
            out.push_str(&format!("{pad}  body\n"));
            for stmt in body {
                dump_stmt(stmt, indent + 2, out);
            }
        }
        Stmt::ForIn {
            binding,
            iterable,
            body,
            ..
        } => {
            out.push_str(&format!("{pad}for {binding}\n"));
            out.push_str(&format!("{pad}  iterable\n"));
            dump_expr(iterable, indent + 2, out);
            out.push_str(&format!("{pad}  body\n"));
            for stmt in body {
                dump_stmt(stmt, indent + 2, out);
            }
        }
        Stmt::ForC {
            init,
            condition,
            step,
            body,
        } => {
            out.push_str(&format!("{pad}for_c\n"));
            out.push_str(&format!("{pad}  init\n"));
            dump_stmt(init, indent + 2, out);
            out.push_str(&format!("{pad}  condition\n"));
            dump_expr(condition, indent + 2, out);
            out.push_str(&format!("{pad}  step\n"));
            dump_stmt(step, indent + 2, out);
            out.push_str(&format!("{pad}  body\n"));
            for stmt in body {
                dump_stmt(stmt, indent + 2, out);
            }
        }
        Stmt::Match { value, arms } => {
            out.push_str(&format!("{pad}match\n"));
            out.push_str(&format!("{pad}  value\n"));
            dump_expr(value, indent + 2, out);
            for arm in arms {
                out.push_str(&format!("{pad}  arm {}\n", pattern_text(&arm.pattern)));
                for stmt in &arm.body {
                    dump_stmt(stmt, indent + 2, out);
                }
            }
        }
        Stmt::Return(Some(value)) => {
            out.push_str(&format!("{pad}return\n"));
            dump_expr(value, indent + 1, out);
        }
        Stmt::Return(None) => {
            out.push_str(&format!("{pad}return Unit\n"));
        }
        Stmt::Break { .. } => {
            out.push_str(&format!("{pad}break\n"));
        }
        Stmt::Continue { .. } => {
            out.push_str(&format!("{pad}continue\n"));
        }
        Stmt::Expr(value) => {
            out.push_str(&format!("{pad}expr\n"));
            dump_expr(value, indent + 1, out);
        }
    }
}

fn dump_expr(expr: &Expr, indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    match expr {
        Expr::Name { name, .. } => out.push_str(&format!("{pad}local {name}\n")),
        Expr::Int { value, .. } => out.push_str(&format!("{pad}const Int {value}\n")),
        Expr::String { value, .. } => out.push_str(&format!("{pad}const String {value:?}\n")),
        Expr::Bool { value, .. } => out.push_str(&format!("{pad}const Bool {value}\n")),
        Expr::Binary {
            op, left, right, ..
        } => {
            out.push_str(&format!("{pad}binary {}\n", binary_op_text(*op)));
            dump_expr(left, indent + 1, out);
            dump_expr(right, indent + 1, out);
        }
        Expr::Unary { op, expr, .. } => {
            out.push_str(&format!("{pad}unary {}\n", unary_op_text(*op)));
            dump_expr(expr, indent + 1, out);
        }
        Expr::Call { callee, args, .. } => {
            out.push_str(&format!("{pad}call {callee}\n"));
            for arg in args {
                dump_expr(arg, indent + 1, out);
            }
        }
        Expr::StructLiteral { ty, fields, .. } => {
            out.push_str(&format!("{pad}struct_literal {ty}\n"));
            for field in fields {
                out.push_str(&format!("{pad}  field {}\n", field.name));
                dump_expr(&field.value, indent + 2, out);
            }
        }
        Expr::FieldAccess { base, field, .. } => {
            out.push_str(&format!("{pad}field_access {field}\n"));
            dump_expr(base, indent + 1, out);
        }
        Expr::ArrayLiteral { elements, .. } => {
            out.push_str(&format!("{pad}array_literal\n"));
            for element in elements {
                dump_expr(element, indent + 1, out);
            }
        }
        Expr::Index { base, index, .. } => {
            out.push_str(&format!("{pad}index\n"));
            dump_expr(base, indent + 1, out);
            dump_expr(index, indent + 1, out);
        }
        Expr::Range { start, end, .. } => {
            out.push_str(&format!("{pad}range\n"));
            dump_expr(start, indent + 1, out);
            dump_expr(end, indent + 1, out);
        }
    }
}

fn pattern_text(pattern: &Pattern) -> String {
    match pattern {
        Pattern::Name(name) => format!("name:{name}"),
        Pattern::Variant {
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
        Pattern::Int(value) => format!("int:{value}"),
        Pattern::String(value) => format!("string:{value:?}"),
        Pattern::Bool(value) => format!("bool:{value}"),
        Pattern::Wildcard => "_".to_string(),
    }
}

fn binary_op_text(op: BinaryOp) -> &'static str {
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

fn unary_op_text(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Not => "not",
        UnaryOp::Neg => "neg",
        UnaryOp::BitNot => "bit_not",
    }
}

fn visibility(exported: bool) -> &'static str {
    if exported {
        "exported"
    } else {
        "private"
    }
}
