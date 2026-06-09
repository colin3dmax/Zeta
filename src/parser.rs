use crate::ast::{
    BinaryOp, EnumDecl, EnumVariant, Expr, Field, Function, Item, MatchArm, Module, Param, Pattern,
    Stmt, StructDecl, StructExprField, UnaryOp,
};
use crate::diagnostic::{Diagnostic, Span};
use crate::lexer::{Keyword, Symbol, Token, TokenKind};

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    allow_struct_literals: bool,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            pos: 0,
            allow_struct_literals: true,
        }
    }

    pub fn parse_module(&mut self) -> Result<Module, Vec<Diagnostic>> {
        let mut items = Vec::new();
        let mut diagnostics = Vec::new();

        while !self.at_eof() {
            match self.parse_item() {
                Ok(item) => items.push(item),
                Err(diagnostic) => {
                    diagnostics.push(diagnostic);
                    self.recover_to_item_boundary();
                }
            }
        }

        if diagnostics.is_empty() {
            Ok(Module { items })
        } else {
            Err(diagnostics)
        }
    }

    fn parse_item(&mut self) -> Result<Item, Diagnostic> {
        if self.consume_keyword(Keyword::Module).is_some() {
            let (path, name_span) = self.parse_path_span()?;
            let name = path.join(".");
            self.expect_symbol(Symbol::Semicolon, "expected `;` after module declaration")?;
            return Ok(Item::ModuleDecl { name, name_span });
        }

        let exported = self.consume_keyword(Keyword::Export).is_some();

        if self.consume_keyword(Keyword::Import).is_some() {
            let (path, path_span) = self.parse_path_span()?;
            let (alias, alias_span) = if self.consume_keyword(Keyword::As).is_some() {
                let (name, span) = self.expect_ident_span("expected import alias after `as`")?;
                (Some(name), Some(span))
            } else {
                (None, None)
            };
            self.expect_symbol(Symbol::Semicolon, "expected `;` after import")?;
            return Ok(Item::Import {
                exported,
                path,
                path_span,
                alias,
                alias_span,
            });
        }

        if self.consume_keyword(Keyword::Struct).is_some() {
            return self.parse_struct(exported).map(Item::Struct);
        }
        if self.consume_keyword(Keyword::Enum).is_some() {
            return self.parse_enum(exported).map(Item::Enum);
        }
        if self.consume_keyword(Keyword::Fn).is_some() {
            return self.parse_function(exported).map(Item::Function);
        }

        Err(self.error_here(
            "PARSE_EXPECTED_ITEM",
            "expected module, import, export, struct, enum, or fn",
        ))
    }

    fn parse_struct(&mut self, exported: bool) -> Result<StructDecl, Diagnostic> {
        let (name, name_span) = self.expect_ident_span("expected struct name")?;
        self.expect_symbol(Symbol::LBrace, "expected `{` after struct name")?;
        let mut fields = Vec::new();
        while !self.check_symbol(Symbol::RBrace) && !self.at_eof() {
            let (field_name, field_name_span) = self.expect_ident_span("expected field name")?;
            self.expect_symbol(Symbol::Colon, "expected `:` after field name")?;
            let (ty, ty_span) = self.expect_ident_span("expected field type")?;
            fields.push(Field {
                name: field_name,
                name_span: field_name_span,
                ty,
                ty_span,
            });
            if self.consume_symbol(Symbol::Comma).is_none() {
                break;
            }
        }
        self.expect_symbol(Symbol::RBrace, "expected `}` after struct fields")?;
        Ok(StructDecl {
            exported,
            name,
            name_span,
            fields,
        })
    }

    fn parse_enum(&mut self, exported: bool) -> Result<EnumDecl, Diagnostic> {
        let (name, name_span) = self.expect_ident_span("expected enum name")?;
        self.expect_symbol(Symbol::LBrace, "expected `{` after enum name")?;
        let mut variants = Vec::new();
        while !self.check_symbol(Symbol::RBrace) && !self.at_eof() {
            let (variant_name, variant_span) =
                self.expect_ident_span("expected enum variant name")?;
            let (payload_type, payload_type_span) = if self.consume_symbol(Symbol::LParen).is_some()
            {
                let (ty, ty_span) = self.expect_ident_span("expected enum variant payload type")?;
                self.expect_symbol(Symbol::RParen, "expected `)` after enum variant payload")?;
                (Some(ty), Some(ty_span))
            } else {
                (None, None)
            };
            variants.push(EnumVariant {
                name: variant_name,
                name_span: variant_span,
                payload_type,
                payload_type_span,
            });
            if self.consume_symbol(Symbol::Comma).is_none() {
                break;
            }
        }
        self.expect_symbol(Symbol::RBrace, "expected `}` after enum variants")?;
        Ok(EnumDecl {
            exported,
            name,
            name_span,
            variants,
        })
    }

    fn parse_function(&mut self, exported: bool) -> Result<Function, Diagnostic> {
        let (name, name_span) = self.expect_ident_span("expected function name")?;
        self.expect_symbol(Symbol::LParen, "expected `(` after function name")?;
        let params = self.parse_params()?;
        self.expect_symbol(Symbol::RParen, "expected `)` after parameters")?;
        let (return_type, return_type_span) = if self.consume_symbol(Symbol::Arrow).is_some() {
            let (ty, ty_span) = self.expect_ident_span("expected return type after `->`")?;
            (Some(ty), Some(ty_span))
        } else {
            (None, None)
        };
        self.expect_symbol(Symbol::LBrace, "expected function body")?;
        let body = self.parse_block_body()?;
        Ok(Function {
            exported,
            name,
            name_span,
            params,
            return_type,
            return_type_span,
            body,
        })
    }

    fn parse_params(&mut self) -> Result<Vec<Param>, Diagnostic> {
        let mut params = Vec::new();
        if self.check_symbol(Symbol::RParen) {
            return Ok(params);
        }
        loop {
            let (name, name_span) = self.expect_ident_span("expected parameter name")?;
            self.expect_symbol(Symbol::Colon, "expected `:` after parameter name")?;
            let (ty, ty_span) = self.expect_ident_span("expected parameter type")?;
            params.push(Param {
                name,
                name_span,
                ty,
                ty_span,
            });
            if self.consume_symbol(Symbol::Comma).is_none() {
                break;
            }
        }
        Ok(params)
    }

    fn parse_path_span(&mut self) -> Result<(Vec<String>, Span), Diagnostic> {
        let start = self.peek().span.start;
        let mut path = vec![self.expect_ident("expected module path")?];
        let mut end = self.tokens[self.pos.saturating_sub(1)].span.end;
        while self.consume_symbol(Symbol::Dot).is_some() {
            path.push(self.expect_ident("expected name after `.`")?);
            end = self.tokens[self.pos.saturating_sub(1)].span.end;
        }
        Ok((path, Span::new(start, end)))
    }

    fn parse_block_body(&mut self) -> Result<Vec<Stmt>, Diagnostic> {
        let mut body = Vec::new();
        while !self.check_symbol(Symbol::RBrace) && !self.at_eof() {
            body.push(self.parse_stmt()?);
        }
        self.expect_symbol(Symbol::RBrace, "expected `}` after function body")?;
        Ok(body)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, Diagnostic> {
        if self.consume_keyword(Keyword::Let).is_some() {
            let mutable = self.consume_keyword(Keyword::Mut).is_some();
            let (name, name_span) = self.expect_ident_span("expected local name")?;
            let (ty, ty_span) = if self.consume_symbol(Symbol::Colon).is_some() {
                let (ty, ty_span) = self.expect_ident_span("expected local type")?;
                (Some(ty), Some(ty_span))
            } else {
                (None, None)
            };
            self.expect_symbol(Symbol::Eq, "expected `=` in let statement")?;
            let value = self.parse_expr()?;
            self.expect_symbol(Symbol::Semicolon, "expected `;` after let statement")?;
            return Ok(Stmt::Let {
                mutable,
                name,
                name_span,
                ty,
                ty_span,
                value,
            });
        }

        if self.consume_keyword(Keyword::If).is_some() {
            let condition = self.parse_expr_without_struct_literals()?;
            self.expect_symbol(Symbol::LBrace, "expected `{` after if condition")?;
            let then_body = self.parse_block_body()?;
            let else_body = if self.consume_keyword(Keyword::Else).is_some() {
                if self.check_keyword(Keyword::If) {
                    // `else if` desugars to `else { if ... }`:嵌套 if 作为单个 else 语句。
                    vec![self.parse_stmt()?]
                } else {
                    self.expect_symbol(Symbol::LBrace, "expected `{` after else")?;
                    self.parse_block_body()?
                }
            } else {
                Vec::new()
            };
            return Ok(Stmt::If {
                condition,
                then_body,
                else_body,
            });
        }

        if self.consume_keyword(Keyword::While).is_some() {
            let condition = self.parse_expr_without_struct_literals()?;
            self.expect_symbol(Symbol::LBrace, "expected `{` after while condition")?;
            let body = self.parse_block_body()?;
            return Ok(Stmt::While { condition, body });
        }

        if self.consume_keyword(Keyword::Match).is_some() {
            let value = self.parse_expr_without_struct_literals()?;
            self.expect_symbol(Symbol::LBrace, "expected `{` after match value")?;
            let mut arms = Vec::new();
            while !self.check_symbol(Symbol::RBrace) && !self.at_eof() {
                arms.push(self.parse_match_arm()?);
            }
            self.expect_symbol(Symbol::RBrace, "expected `}` after match arms")?;
            return Ok(Stmt::Match { value, arms });
        }

        if self.consume_keyword(Keyword::Return).is_some() {
            if self.consume_symbol(Symbol::Semicolon).is_some() {
                return Ok(Stmt::Return(None));
            }
            let value = self.parse_expr()?;
            self.expect_symbol(Symbol::Semicolon, "expected `;` after return statement")?;
            return Ok(Stmt::Return(Some(value)));
        }

        if let Some(span) = self.consume_keyword(Keyword::Break) {
            self.expect_symbol(Symbol::Semicolon, "expected `;` after break statement")?;
            return Ok(Stmt::Break { span });
        }

        if let Some(span) = self.consume_keyword(Keyword::Continue) {
            self.expect_symbol(Symbol::Semicolon, "expected `;` after continue statement")?;
            return Ok(Stmt::Continue { span });
        }

        let expr = self.parse_expr()?;
        if self.consume_symbol(Symbol::Eq).is_some() {
            let value = self.parse_expr()?;
            self.expect_symbol(Symbol::Semicolon, "expected `;` after assignment")?;
            return Ok(Stmt::Assign {
                target: expr,
                value,
            });
        }
        if let Some(op) = self.consume_compound_assign_op() {
            let rhs = self.parse_expr()?;
            self.expect_symbol(Symbol::Semicolon, "expected `;` after assignment")?;
            let span = Span::new(expr.span().start, rhs.span().end);
            // `a += b` desugars to `a = a + b`(左值在 dump/求值中出现两次)。
            let value = Expr::Binary {
                op,
                left: Box::new(expr.clone()),
                right: Box::new(rhs),
                span,
            };
            return Ok(Stmt::Assign { target: expr, value });
        }
        self.expect_symbol(Symbol::Semicolon, "expected `;` after expression statement")?;
        Ok(Stmt::Expr(expr))
    }

    fn consume_compound_assign_op(&mut self) -> Option<BinaryOp> {
        if self.consume_symbol(Symbol::PlusEq).is_some() {
            return Some(BinaryOp::Add);
        }
        if self.consume_symbol(Symbol::MinusEq).is_some() {
            return Some(BinaryOp::Sub);
        }
        if self.consume_symbol(Symbol::StarEq).is_some() {
            return Some(BinaryOp::Mul);
        }
        if self.consume_symbol(Symbol::SlashEq).is_some() {
            return Some(BinaryOp::Div);
        }
        if self.consume_symbol(Symbol::PercentEq).is_some() {
            return Some(BinaryOp::Mod);
        }
        None
    }

    fn parse_match_arm(&mut self) -> Result<MatchArm, Diagnostic> {
        let pattern = self.parse_pattern()?;
        self.expect_symbol(Symbol::Arrow, "expected `->` after match pattern")?;
        self.expect_symbol(Symbol::LBrace, "expected `{` after match arm")?;
        let body = self.parse_block_body()?;
        if self.consume_symbol(Symbol::Comma).is_none() {
            return Ok(MatchArm { pattern, body });
        }
        Ok(MatchArm { pattern, body })
    }

    fn parse_pattern(&mut self) -> Result<Pattern, Diagnostic> {
        match self.peek_kind() {
            TokenKind::Ident(name) if name == "_" => {
                self.pos += 1;
                Ok(Pattern::Wildcard)
            }
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.pos += 1;
                if self.consume_symbol(Symbol::Dot).is_some() {
                    let variant = self.expect_ident("expected enum variant name after `.`")?;
                    let binding = if self.consume_symbol(Symbol::LParen).is_some() {
                        let binding = self.expect_ident("expected variant payload binding")?;
                        self.expect_symbol(Symbol::RParen, "expected `)` after variant pattern")?;
                        Some(binding)
                    } else {
                        None
                    };
                    return Ok(Pattern::Variant {
                        enum_name: name,
                        variant,
                        binding,
                    });
                }
                Ok(Pattern::Name(name))
            }
            TokenKind::Int(value) => {
                let value = value.clone();
                self.pos += 1;
                Ok(Pattern::Int(value))
            }
            TokenKind::String(value) => {
                let value = value.clone();
                self.pos += 1;
                Ok(Pattern::String(value))
            }
            TokenKind::Keyword(Keyword::True) => {
                self.pos += 1;
                Ok(Pattern::Bool(true))
            }
            TokenKind::Keyword(Keyword::False) => {
                self.pos += 1;
                Ok(Pattern::Bool(false))
            }
            _ => Err(self.error_here("PARSE_EXPECTED_PATTERN", "expected match pattern")),
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, Diagnostic> {
        self.parse_logical_or()
    }

    fn parse_expr_without_struct_literals(&mut self) -> Result<Expr, Diagnostic> {
        let previous = self.allow_struct_literals;
        self.allow_struct_literals = false;
        let parsed = self.parse_expr();
        self.allow_struct_literals = previous;
        parsed
    }

    fn parse_logical_or(&mut self) -> Result<Expr, Diagnostic> {
        let mut expr = self.parse_logical_and()?;
        while self.consume_symbol(Symbol::OrOr).is_some() {
            let right = self.parse_logical_and()?;
            let span = Span::new(expr.span().start, right.span().end);
            expr = Expr::Binary {
                op: BinaryOp::Or,
                left: Box::new(expr),
                right: Box::new(right),
                span,
            };
        }
        Ok(expr)
    }

    fn parse_logical_and(&mut self) -> Result<Expr, Diagnostic> {
        let mut expr = self.parse_comparison()?;
        while self.consume_symbol(Symbol::AndAnd).is_some() {
            let right = self.parse_comparison()?;
            let span = Span::new(expr.span().start, right.span().end);
            expr = Expr::Binary {
                op: BinaryOp::And,
                left: Box::new(expr),
                right: Box::new(right),
                span,
            };
        }
        Ok(expr)
    }

    fn parse_comparison(&mut self) -> Result<Expr, Diagnostic> {
        let mut expr = self.parse_additive()?;
        loop {
            let op = if self.consume_symbol(Symbol::EqEq).is_some() {
                BinaryOp::Eq
            } else if self.consume_symbol(Symbol::BangEq).is_some() {
                BinaryOp::NotEq
            } else if self.consume_symbol(Symbol::Lte).is_some() {
                BinaryOp::Lte
            } else if self.consume_symbol(Symbol::Lt).is_some() {
                BinaryOp::Lt
            } else if self.consume_symbol(Symbol::Gte).is_some() {
                BinaryOp::Gte
            } else if self.consume_symbol(Symbol::Gt).is_some() {
                BinaryOp::Gt
            } else {
                break;
            };
            let right = self.parse_additive()?;
            let span = Span::new(expr.span().start, right.span().end);
            expr = Expr::Binary {
                op,
                left: Box::new(expr),
                right: Box::new(right),
                span,
            };
        }
        Ok(expr)
    }

    fn parse_additive(&mut self) -> Result<Expr, Diagnostic> {
        let mut expr = self.parse_multiplicative()?;
        loop {
            let op = if self.consume_symbol(Symbol::Plus).is_some() {
                BinaryOp::Add
            } else if self.consume_symbol(Symbol::Minus).is_some() {
                BinaryOp::Sub
            } else {
                break;
            };
            let right = self.parse_multiplicative()?;
            let span = Span::new(expr.span().start, right.span().end);
            expr = Expr::Binary {
                op,
                left: Box::new(expr),
                right: Box::new(right),
                span,
            };
        }
        Ok(expr)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, Diagnostic> {
        let mut expr = self.parse_unary()?;
        loop {
            let op = if self.consume_symbol(Symbol::Star).is_some() {
                BinaryOp::Mul
            } else if self.consume_symbol(Symbol::Slash).is_some() {
                BinaryOp::Div
            } else if self.consume_symbol(Symbol::Percent).is_some() {
                BinaryOp::Mod
            } else {
                break;
            };
            let right = self.parse_unary()?;
            let span = Span::new(expr.span().start, right.span().end);
            expr = Expr::Binary {
                op,
                left: Box::new(expr),
                right: Box::new(right),
                span,
            };
        }
        Ok(expr)
    }

    fn parse_unary(&mut self) -> Result<Expr, Diagnostic> {
        if let Some(start) = self.consume_symbol(Symbol::Bang) {
            let expr = self.parse_unary()?;
            let span = Span::new(start.start, expr.span().end);
            return Ok(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(expr),
                span,
            });
        }
        if let Some(start) = self.consume_symbol(Symbol::Minus) {
            let expr = self.parse_unary()?;
            let span = Span::new(start.start, expr.span().end);
            return Ok(Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(expr),
                span,
            });
        }
        self.parse_call()
    }

    fn parse_call(&mut self) -> Result<Expr, Diagnostic> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.consume_symbol(Symbol::LParen).is_some() {
                let Some((callee, callee_span)) = expr_path(&expr) else {
                    return Err(self.error_here(
                        "PARSE_EXPECTED_CALL_TARGET",
                        "expected function name before call arguments",
                    ));
                };
                let args = self.parse_call_args()?;
                let end = self.previous_span().end;
                expr = Expr::Call {
                    callee,
                    callee_span,
                    args,
                    span: Span::new(callee_span.start, end),
                };
                continue;
            }

            if self.allow_struct_literals
                && matches!(expr, Expr::Name { .. })
                && self.check_symbol(Symbol::LBrace)
            {
                self.expect_symbol(Symbol::LBrace, "expected `{` after struct type")?;
                let Expr::Name { name: ty, span } = expr else {
                    unreachable!("struct literal target was checked above");
                };
                let fields = self.parse_struct_expr_fields()?;
                let end = self.previous_span().end;
                expr = Expr::StructLiteral {
                    ty,
                    ty_span: span,
                    fields,
                    span: Span::new(span.start, end),
                };
                continue;
            }

            if self.consume_symbol(Symbol::Dot).is_some() {
                let (field, field_span) =
                    self.expect_ident_span("expected field name after `.`")?;
                let span = Span::new(expr.span().start, field_span.end);
                expr = Expr::FieldAccess {
                    base: Box::new(expr),
                    field,
                    field_span,
                    span,
                };
                continue;
            }

            if self.consume_symbol(Symbol::LBracket).is_some() {
                let start = expr.span().start;
                let index = self.parse_expr()?;
                self.expect_symbol(Symbol::RBracket, "expected `]` after index expression")?;
                let end = self.previous_span().end;
                expr = Expr::Index {
                    base: Box::new(expr),
                    index: Box::new(index),
                    span: Span::new(start, end),
                };
                continue;
            }

            return Ok(expr);
        }
    }

    fn parse_struct_expr_fields(&mut self) -> Result<Vec<StructExprField>, Diagnostic> {
        let mut fields = Vec::new();
        if self.consume_symbol(Symbol::RBrace).is_some() {
            return Ok(fields);
        }
        loop {
            let (name, name_span) = self.expect_ident_span("expected struct field name")?;
            self.expect_symbol(Symbol::Colon, "expected `:` after struct field name")?;
            let value = self.parse_expr()?;
            fields.push(StructExprField {
                name,
                name_span,
                value,
            });
            if self.consume_symbol(Symbol::Comma).is_none() {
                break;
            }
            if self.consume_symbol(Symbol::RBrace).is_some() {
                return Ok(fields);
            }
        }
        self.expect_symbol(Symbol::RBrace, "expected `}` after struct literal fields")?;
        Ok(fields)
    }

    fn parse_call_args(&mut self) -> Result<Vec<Expr>, Diagnostic> {
        let mut args = Vec::new();
        if self.consume_symbol(Symbol::RParen).is_some() {
            return Ok(args);
        }
        loop {
            args.push(self.parse_expr()?);
            if self.consume_symbol(Symbol::Comma).is_none() {
                break;
            }
        }
        self.expect_symbol(Symbol::RParen, "expected `)` after call arguments")?;
        Ok(args)
    }

    fn parse_primary(&mut self) -> Result<Expr, Diagnostic> {
        match self.peek_kind() {
            TokenKind::Ident(name) => {
                let name = name.clone();
                let span = self.peek().span;
                self.pos += 1;
                Ok(Expr::Name { name, span })
            }
            TokenKind::Int(value) => {
                let value = value.clone();
                let span = self.peek().span;
                self.pos += 1;
                Ok(Expr::Int { value, span })
            }
            TokenKind::String(value) => {
                let value = value.clone();
                let span = self.peek().span;
                self.pos += 1;
                Ok(Expr::String { value, span })
            }
            TokenKind::Keyword(Keyword::True) => {
                let span = self.peek().span;
                self.pos += 1;
                Ok(Expr::Bool { value: true, span })
            }
            TokenKind::Keyword(Keyword::False) => {
                let span = self.peek().span;
                self.pos += 1;
                Ok(Expr::Bool { value: false, span })
            }
            TokenKind::Symbol(Symbol::LParen) => {
                self.pos += 1;
                let expr = self.parse_expr()?;
                self.expect_symbol(Symbol::RParen, "expected `)` after expression")?;
                Ok(expr)
            }
            TokenKind::Symbol(Symbol::LBracket) => self.parse_array_literal(),
            _ => Err(self.error_here("PARSE_EXPECTED_EXPR", "expected expression")),
        }
    }

    fn parse_array_literal(&mut self) -> Result<Expr, Diagnostic> {
        let start = self.peek().span;
        self.expect_symbol(Symbol::LBracket, "expected `[` before array literal")?;
        let mut elements = Vec::new();
        if self.consume_symbol(Symbol::RBracket).is_some() {
            let end = self.previous_span().end;
            return Ok(Expr::ArrayLiteral {
                elements,
                span: Span::new(start.start, end),
            });
        }
        loop {
            elements.push(self.parse_expr()?);
            if self.consume_symbol(Symbol::Comma).is_none() {
                break;
            }
            if self.consume_symbol(Symbol::RBracket).is_some() {
                let end = self.previous_span().end;
                return Ok(Expr::ArrayLiteral {
                    elements,
                    span: Span::new(start.start, end),
                });
            }
        }
        self.expect_symbol(Symbol::RBracket, "expected `]` after array literal")?;
        let end = self.previous_span().end;
        Ok(Expr::ArrayLiteral {
            elements,
            span: Span::new(start.start, end),
        })
    }

    fn recover_to_item_boundary(&mut self) {
        while !self.at_eof() {
            if self.consume_symbol(Symbol::Semicolon).is_some() {
                return;
            }
            if matches!(
                self.peek_kind(),
                TokenKind::Keyword(Keyword::Module)
                    | TokenKind::Keyword(Keyword::Import)
                    | TokenKind::Keyword(Keyword::Export)
                    | TokenKind::Keyword(Keyword::Struct)
                    | TokenKind::Keyword(Keyword::Enum)
                    | TokenKind::Keyword(Keyword::Fn)
            ) {
                return;
            }
            self.pos += 1;
        }
    }

    fn expect_ident(&mut self, message: &'static str) -> Result<String, Diagnostic> {
        self.expect_ident_span(message).map(|(name, _)| name)
    }

    fn expect_ident_span(&mut self, message: &'static str) -> Result<(String, Span), Diagnostic> {
        match self.peek_kind() {
            TokenKind::Ident(name) => {
                let name = name.clone();
                let span = self.peek().span;
                self.pos += 1;
                Ok((name, span))
            }
            _ => Err(self.error_here("PARSE_EXPECTED_IDENT", message)),
        }
    }

    fn expect_symbol(&mut self, symbol: Symbol, message: &'static str) -> Result<(), Diagnostic> {
        if self.consume_symbol(symbol).is_some() {
            Ok(())
        } else {
            Err(self.error_here("PARSE_EXPECTED_SYMBOL", message))
        }
    }

    fn consume_keyword(&mut self, keyword: Keyword) -> Option<Span> {
        if self.check_keyword(keyword) {
            let span = self.peek().span;
            self.pos += 1;
            Some(span)
        } else {
            None
        }
    }

    fn consume_symbol(&mut self, symbol: Symbol) -> Option<Span> {
        if self.check_symbol(symbol) {
            let span = self.peek().span;
            self.pos += 1;
            Some(span)
        } else {
            None
        }
    }

    fn check_keyword(&self, keyword: Keyword) -> bool {
        matches!(self.peek_kind(), TokenKind::Keyword(found) if *found == keyword)
    }

    fn check_symbol(&self, symbol: Symbol) -> bool {
        matches!(self.peek_kind(), TokenKind::Symbol(found) if *found == symbol)
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek_kind(), TokenKind::Eof)
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.peek().kind
    }

    fn previous_span(&self) -> Span {
        self.tokens
            .get(self.pos.saturating_sub(1))
            .map(|token| token.span)
            .unwrap_or_else(|| self.peek().span)
    }

    fn error_here(&self, code: &'static str, message: &'static str) -> Diagnostic {
        Diagnostic::new(code, message, self.peek().span)
    }
}

fn expr_path(expr: &Expr) -> Option<(String, Span)> {
    match expr {
        Expr::Name { name, span } => Some((name.clone(), *span)),
        Expr::FieldAccess {
            base,
            field,
            field_span,
            span,
        } => {
            let (base, _) = expr_path(base)?;
            Some((
                format!("{base}.{field}"),
                Span::new(span.start, field_span.end),
            ))
        }
        _ => None,
    }
}
