use crate::diagnostic::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    ModuleDecl { name: String },
    Import { path: Vec<String> },
    Struct(StructDecl),
    Enum(EnumDecl),
    Function(Function),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructDecl {
    pub exported: bool,
    pub name: String,
    pub name_span: Span,
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub name_span: Span,
    pub ty: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumDecl {
    pub exported: bool,
    pub name: String,
    pub name_span: Span,
    pub variants: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub exported: bool,
    pub name: String,
    pub name_span: Span,
    pub params: Vec<Param>,
    pub return_type: Option<String>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    pub name: String,
    pub name_span: Span,
    pub ty: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt {
    Let {
        mutable: bool,
        name: String,
        name_span: Span,
        ty: Option<String>,
        value: Expr,
    },
    Assign {
        name: String,
        name_span: Span,
        value: Expr,
    },
    If {
        condition: Expr,
        then_body: Vec<Stmt>,
        else_body: Vec<Stmt>,
    },
    While {
        condition: Expr,
        body: Vec<Stmt>,
    },
    Match {
        value: Expr,
        arms: Vec<MatchArm>,
    },
    Return(Option<Expr>),
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    Name(String),
    Int(String),
    String(String),
    Bool(bool),
    Wildcard,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Name {
        name: String,
        span: Span,
    },
    Int {
        value: String,
        span: Span,
    },
    String {
        value: String,
        span: Span,
    },
    Bool {
        value: bool,
        span: Span,
    },
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
        span: Span,
    },
    Call {
        callee: String,
        callee_span: Span,
        args: Vec<Expr>,
        span: Span,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
}

impl Module {
    pub fn dump(&self) -> String {
        let mut out = String::from("Module\n");
        for item in &self.items {
            item.dump(1, &mut out);
        }
        out
    }
}

impl Item {
    fn dump(&self, indent: usize, out: &mut String) {
        let pad = "  ".repeat(indent);
        match self {
            Item::ModuleDecl { name } => {
                out.push_str(&format!("{pad}ModuleDecl name={name}\n"));
            }
            Item::Import { path } => {
                out.push_str(&format!("{pad}Import path={}\n", path.join(".")));
            }
            Item::Struct(decl) => {
                out.push_str(&format!(
                    "{pad}Struct name={} exported={}\n",
                    decl.name, decl.exported
                ));
                for field in &decl.fields {
                    out.push_str(&format!(
                        "{pad}  Field name={} type={}\n",
                        field.name, field.ty
                    ));
                }
            }
            Item::Enum(decl) => {
                out.push_str(&format!(
                    "{pad}Enum name={} exported={}\n",
                    decl.name, decl.exported
                ));
                for variant in &decl.variants {
                    out.push_str(&format!("{pad}  Variant name={variant}\n"));
                }
            }
            Item::Function(function) => {
                out.push_str(&format!(
                    "{pad}Function name={} exported={}\n",
                    function.name, function.exported
                ));
                for param in &function.params {
                    out.push_str(&format!(
                        "{pad}  Param name={} type={}\n",
                        param.name, param.ty
                    ));
                }
                if let Some(return_type) = &function.return_type {
                    out.push_str(&format!("{pad}  Return type={return_type}\n"));
                }
                for stmt in &function.body {
                    stmt.dump(indent + 1, out);
                }
            }
        }
    }
}

impl Stmt {
    fn dump(&self, indent: usize, out: &mut String) {
        let pad = "  ".repeat(indent);
        match self {
            Stmt::Let {
                mutable,
                name,
                ty,
                value,
                ..
            } => {
                out.push_str(&format!("{pad}Let name={name}"));
                if let Some(ty) = ty {
                    out.push_str(&format!(" type={ty}"));
                }
                if *mutable {
                    out.push_str(" mutable=true");
                }
                out.push('\n');
                value.dump(indent + 1, out);
            }
            Stmt::Assign { name, value, .. } => {
                out.push_str(&format!("{pad}Assign name={name}\n"));
                value.dump(indent + 1, out);
            }
            Stmt::If {
                condition,
                then_body,
                else_body,
            } => {
                out.push_str(&format!("{pad}If\n"));
                out.push_str(&format!("{pad}  Condition\n"));
                condition.dump(indent + 2, out);
                out.push_str(&format!("{pad}  Then\n"));
                for stmt in then_body {
                    stmt.dump(indent + 2, out);
                }
                if !else_body.is_empty() {
                    out.push_str(&format!("{pad}  Else\n"));
                    for stmt in else_body {
                        stmt.dump(indent + 2, out);
                    }
                }
            }
            Stmt::While { condition, body } => {
                out.push_str(&format!("{pad}While\n"));
                out.push_str(&format!("{pad}  Condition\n"));
                condition.dump(indent + 2, out);
                out.push_str(&format!("{pad}  Body\n"));
                for stmt in body {
                    stmt.dump(indent + 2, out);
                }
            }
            Stmt::Match { value, arms } => {
                out.push_str(&format!("{pad}Match\n"));
                out.push_str(&format!("{pad}  Value\n"));
                value.dump(indent + 2, out);
                for arm in arms {
                    out.push_str(&format!("{pad}  Arm pattern={}\n", arm.pattern.dump()));
                    for stmt in &arm.body {
                        stmt.dump(indent + 2, out);
                    }
                }
            }
            Stmt::Return(Some(value)) => {
                out.push_str(&format!("{pad}Return\n"));
                value.dump(indent + 1, out);
            }
            Stmt::Return(None) => {
                out.push_str(&format!("{pad}Return\n"));
            }
            Stmt::Expr(value) => {
                out.push_str(&format!("{pad}ExprStmt\n"));
                value.dump(indent + 1, out);
            }
        }
    }
}

impl Pattern {
    fn dump(&self) -> String {
        match self {
            Pattern::Name(name) => format!("name:{name}"),
            Pattern::Int(value) => format!("int:{value}"),
            Pattern::String(value) => format!("string:{value:?}"),
            Pattern::Bool(value) => format!("bool:{value}"),
            Pattern::Wildcard => "_".to_string(),
        }
    }
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Name { span, .. }
            | Expr::Int { span, .. }
            | Expr::String { span, .. }
            | Expr::Bool { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Call { span, .. } => *span,
        }
    }

    fn dump(&self, indent: usize, out: &mut String) {
        let pad = "  ".repeat(indent);
        match self {
            Expr::Name { name, .. } => out.push_str(&format!("{pad}Name {name}\n")),
            Expr::Int { value, .. } => out.push_str(&format!("{pad}Int {value}\n")),
            Expr::String { value, .. } => out.push_str(&format!("{pad}String {value:?}\n")),
            Expr::Bool { value, .. } => out.push_str(&format!("{pad}Bool {value}\n")),
            Expr::Binary {
                op, left, right, ..
            } => {
                out.push_str(&format!("{pad}Binary op={}\n", op.as_str()));
                left.dump(indent + 1, out);
                right.dump(indent + 1, out);
            }
            Expr::Call { callee, args, .. } => {
                out.push_str(&format!("{pad}Call callee={callee}\n"));
                for arg in args {
                    arg.dump(indent + 1, out);
                }
            }
        }
    }
}

impl BinaryOp {
    fn as_str(self) -> &'static str {
        match self {
            BinaryOp::Add => "add",
            BinaryOp::Sub => "sub",
            BinaryOp::Mul => "mul",
            BinaryOp::Div => "div",
        }
    }
}
