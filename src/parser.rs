use crate::ast::{
    BinaryOp, EnumDecl, Expr, Field, Function, Item, MatchArm, Module, Param, Pattern, Stmt,
    StructDecl,
};
use crate::diagnostic::{Diagnostic, Span};
use crate::lexer::{Keyword, Symbol, Token, TokenKind};

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
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
            let name = self.expect_ident("expected module name")?;
            self.expect_symbol(Symbol::Semicolon, "expected `;` after module declaration")?;
            return Ok(Item::ModuleDecl { name });
        }

        if self.consume_keyword(Keyword::Import).is_some() {
            let path = self.parse_path()?;
            self.expect_symbol(Symbol::Semicolon, "expected `;` after import")?;
            return Ok(Item::Import { path });
        }

        let exported = self.consume_keyword(Keyword::Export).is_some();
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
        let name = self.expect_ident("expected struct name")?;
        self.expect_symbol(Symbol::LBrace, "expected `{` after struct name")?;
        let mut fields = Vec::new();
        while !self.check_symbol(Symbol::RBrace) && !self.at_eof() {
            let field_name = self.expect_ident("expected field name")?;
            self.expect_symbol(Symbol::Colon, "expected `:` after field name")?;
            let ty = self.expect_ident("expected field type")?;
            fields.push(Field {
                name: field_name,
                ty,
            });
            if self.consume_symbol(Symbol::Comma).is_none() {
                break;
            }
        }
        self.expect_symbol(Symbol::RBrace, "expected `}` after struct fields")?;
        Ok(StructDecl {
            exported,
            name,
            fields,
        })
    }

    fn parse_enum(&mut self, exported: bool) -> Result<EnumDecl, Diagnostic> {
        let name = self.expect_ident("expected enum name")?;
        self.expect_symbol(Symbol::LBrace, "expected `{` after enum name")?;
        let mut variants = Vec::new();
        while !self.check_symbol(Symbol::RBrace) && !self.at_eof() {
            variants.push(self.expect_ident("expected enum variant name")?);
            if self.consume_symbol(Symbol::Comma).is_none() {
                break;
            }
        }
        self.expect_symbol(Symbol::RBrace, "expected `}` after enum variants")?;
        Ok(EnumDecl {
            exported,
            name,
            variants,
        })
    }

    fn parse_function(&mut self, exported: bool) -> Result<Function, Diagnostic> {
        let name = self.expect_ident("expected function name")?;
        self.expect_symbol(Symbol::LParen, "expected `(` after function name")?;
        let params = self.parse_params()?;
        self.expect_symbol(Symbol::RParen, "expected `)` after parameters")?;
        let return_type = if self.consume_symbol(Symbol::Arrow).is_some() {
            Some(self.expect_ident("expected return type after `->`")?)
        } else {
            None
        };
        self.expect_symbol(Symbol::LBrace, "expected function body")?;
        let body = self.parse_block_body()?;
        Ok(Function {
            exported,
            name,
            params,
            return_type,
            body,
        })
    }

    fn parse_params(&mut self) -> Result<Vec<Param>, Diagnostic> {
        let mut params = Vec::new();
        if self.check_symbol(Symbol::RParen) {
            return Ok(params);
        }
        loop {
            let name = self.expect_ident("expected parameter name")?;
            self.expect_symbol(Symbol::Colon, "expected `:` after parameter name")?;
            let ty = self.expect_ident("expected parameter type")?;
            params.push(Param { name, ty });
            if self.consume_symbol(Symbol::Comma).is_none() {
                break;
            }
        }
        Ok(params)
    }

    fn parse_path(&mut self) -> Result<Vec<String>, Diagnostic> {
        let mut path = vec![self.expect_ident("expected import path")?];
        while self.consume_symbol(Symbol::Dot).is_some() {
            path.push(self.expect_ident("expected name after `.`")?);
        }
        Ok(path)
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
            let name = self.expect_ident("expected local name")?;
            let ty = if self.consume_symbol(Symbol::Colon).is_some() {
                Some(self.expect_ident("expected local type")?)
            } else {
                None
            };
            self.expect_symbol(Symbol::Eq, "expected `=` in let statement")?;
            let value = self.parse_expr()?;
            self.expect_symbol(Symbol::Semicolon, "expected `;` after let statement")?;
            return Ok(Stmt::Let { name, ty, value });
        }

        if self.consume_keyword(Keyword::If).is_some() {
            let condition = self.parse_expr()?;
            self.expect_symbol(Symbol::LBrace, "expected `{` after if condition")?;
            let then_body = self.parse_block_body()?;
            let else_body = if self.consume_keyword(Keyword::Else).is_some() {
                self.expect_symbol(Symbol::LBrace, "expected `{` after else")?;
                self.parse_block_body()?
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
            let condition = self.parse_expr()?;
            self.expect_symbol(Symbol::LBrace, "expected `{` after while condition")?;
            let body = self.parse_block_body()?;
            return Ok(Stmt::While { condition, body });
        }

        if self.consume_keyword(Keyword::Match).is_some() {
            let value = self.parse_expr()?;
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

        let value = self.parse_expr()?;
        self.expect_symbol(Symbol::Semicolon, "expected `;` after expression statement")?;
        Ok(Stmt::Expr(value))
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
        self.parse_additive()
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
            expr = Expr::Binary {
                op,
                left: Box::new(expr),
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, Diagnostic> {
        let mut expr = self.parse_primary()?;
        loop {
            let op = if self.consume_symbol(Symbol::Star).is_some() {
                BinaryOp::Mul
            } else if self.consume_symbol(Symbol::Slash).is_some() {
                BinaryOp::Div
            } else {
                break;
            };
            let right = self.parse_primary()?;
            expr = Expr::Binary {
                op,
                left: Box::new(expr),
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, Diagnostic> {
        match self.peek_kind() {
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.pos += 1;
                Ok(Expr::Name(name))
            }
            TokenKind::Int(value) => {
                let value = value.clone();
                self.pos += 1;
                Ok(Expr::Int(value))
            }
            TokenKind::String(value) => {
                let value = value.clone();
                self.pos += 1;
                Ok(Expr::String(value))
            }
            TokenKind::Keyword(Keyword::True) => {
                self.pos += 1;
                Ok(Expr::Bool(true))
            }
            TokenKind::Keyword(Keyword::False) => {
                self.pos += 1;
                Ok(Expr::Bool(false))
            }
            TokenKind::Symbol(Symbol::LParen) => {
                self.pos += 1;
                let expr = self.parse_expr()?;
                self.expect_symbol(Symbol::RParen, "expected `)` after expression")?;
                Ok(expr)
            }
            _ => Err(self.error_here("PARSE_EXPECTED_EXPR", "expected expression")),
        }
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
        match self.peek_kind() {
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.pos += 1;
                Ok(name)
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

    fn error_here(&self, code: &'static str, message: &'static str) -> Diagnostic {
        Diagnostic::new(code, message, self.peek().span)
    }
}
