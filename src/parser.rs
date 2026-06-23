use crate::ast::{
    BinaryOp, EnumDecl, EnumVariant, Expr, Field, Function, ImplBlock, Item, MatchArm, Module,
    Param, Pattern, Stmt, StructDecl, StructExprField, TraitBound, TraitDecl, TraitMethod, UnaryOp,
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
        // `reloadable` is a contextual modifier (only meaningful right before
        // `fn`), so it is NOT a reserved keyword — this keeps the lexer/token-kind
        // numbering identical and avoids any self-hosting parity churn.
        let reloadable = self.consume_reloadable();
        // `extern fn name(..) -> ..;` — a bodyless external (C ABI) declaration.
        // Contextual modifier (not a reserved keyword), same precedent as
        // `reloadable`/`trait`, so the lexer/token numbering is untouched.
        let is_extern = self.consume_contextual_before_fn("extern");

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
            return self
                .parse_function(exported, reloadable, is_extern)
                .map(Item::Function);
        }
        // `trait` / `impl` are contextual identifiers (not reserved keywords), so
        // the lexer/token-kind numbering stays identical and self-hosting parity
        // is untouched — same precedent as `reloadable`.
        if self.consume_contextual("trait") {
            return self.parse_trait(exported).map(Item::Trait);
        }
        if self.consume_contextual("impl") {
            return self.parse_impl(exported).map(Item::Impl);
        }

        Err(self.error_here(
            "PARSE_EXPECTED_ITEM",
            "expected module, import, export, struct, enum, trait, impl, or fn",
        ))
    }

    fn parse_struct(&mut self, exported: bool) -> Result<StructDecl, Diagnostic> {
        let (name, name_span) = self.expect_ident_span("expected struct name")?;
        let type_params = self.parse_type_params()?;
        self.expect_symbol(Symbol::LBrace, "expected `{` after struct name")?;
        let mut fields = Vec::new();
        while !self.check_symbol(Symbol::RBrace) && !self.at_eof() {
            let (field_name, field_name_span) = self.expect_ident_span("expected field name")?;
            self.expect_symbol(Symbol::Colon, "expected `:` after field name")?;
            let (ty, ty_span) = self.parse_type_annotation("expected field type")?;
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
            type_params,
            fields,
        })
    }

    fn parse_enum(&mut self, exported: bool) -> Result<EnumDecl, Diagnostic> {
        let (name, name_span) = self.expect_ident_span("expected enum name")?;
        let type_params = self.parse_type_params()?;
        self.expect_symbol(Symbol::LBrace, "expected `{` after enum name")?;
        let mut variants = Vec::new();
        while !self.check_symbol(Symbol::RBrace) && !self.at_eof() {
            let (variant_name, variant_span) =
                self.expect_ident_span("expected enum variant name")?;
            let (payload_type, payload_type_span) = if self.consume_symbol(Symbol::LParen).is_some()
            {
                let (ty, ty_span) =
                    self.parse_type_annotation("expected enum variant payload type")?;
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
            type_params,
            variants,
        })
    }

    fn parse_trait(&mut self, exported: bool) -> Result<TraitDecl, Diagnostic> {
        let (name, name_span) = self.expect_ident_span("expected trait name")?;
        let type_params = self.parse_type_params()?;
        self.expect_symbol(Symbol::LBrace, "expected `{` after trait name")?;
        let mut methods = Vec::new();
        while !self.check_symbol(Symbol::RBrace) && !self.at_eof() {
            if self.consume_keyword(Keyword::Fn).is_none() {
                return Err(self.error_here(
                    "PARSE_EXPECTED_TRAIT_METHOD",
                    "expected `fn` for trait method signature",
                ));
            }
            let (method_name, method_name_span) = self.expect_ident_span("expected method name")?;
            let method_type_params = self.parse_type_params()?;
            self.expect_symbol(Symbol::LParen, "expected `(` after method name")?;
            let params = self.parse_params()?;
            self.expect_symbol(Symbol::RParen, "expected `)` after method parameters")?;
            let (return_type, return_type_span) = if self.consume_symbol(Symbol::Arrow).is_some() {
                let (ty, ty_span) =
                    self.parse_type_annotation("expected return type after `->`")?;
                (Some(ty), Some(ty_span))
            } else {
                (None, None)
            };
            self.expect_symbol(
                Symbol::Semicolon,
                "expected `;` after trait method signature",
            )?;
            methods.push(TraitMethod {
                name: method_name,
                name_span: method_name_span,
                type_params: method_type_params,
                params,
                return_type,
                return_type_span,
            });
        }
        self.expect_symbol(Symbol::RBrace, "expected `}` after trait methods")?;
        Ok(TraitDecl {
            exported,
            name,
            name_span,
            type_params,
            methods,
        })
    }

    fn parse_impl(&mut self, exported: bool) -> Result<ImplBlock, Diagnostic> {
        let type_params = self.parse_type_params()?;
        // The trait reference may carry generic args (`Container<T>`), so parse it
        // as a type annotation; it stops at the `for` keyword. Later slices strip
        // to the base name via `type_syntax::base_name` for dispatch.
        let (trait_name, trait_name_span) =
            self.parse_type_annotation("expected trait name in impl")?;
        if self.consume_keyword(Keyword::For).is_none() {
            return Err(self.error_here(
                "PARSE_EXPECTED_FOR",
                "expected `for` in impl header",
            ));
        }
        let (target_type, target_type_span) =
            self.parse_type_annotation("expected target type after `for`")?;
        self.expect_symbol(Symbol::LBrace, "expected `{` after impl header")?;
        let mut methods = Vec::new();
        while !self.check_symbol(Symbol::RBrace) && !self.at_eof() {
            let fn_exported = self.consume_keyword(Keyword::Export).is_some();
            if self.consume_keyword(Keyword::Fn).is_none() {
                return Err(self.error_here(
                    "PARSE_EXPECTED_IMPL_METHOD",
                    "expected `fn` for impl method",
                ));
            }
            methods.push(self.parse_function(fn_exported, false, false)?);
        }
        self.expect_symbol(Symbol::RBrace, "expected `}` after impl methods")?;
        Ok(ImplBlock {
            exported,
            type_params,
            trait_name,
            trait_name_span,
            target_type,
            target_type_span,
            methods,
        })
    }

    fn parse_function(
        &mut self,
        exported: bool,
        reloadable: bool,
        is_extern: bool,
    ) -> Result<Function, Diagnostic> {
        let (name, name_span) = self.expect_ident_span("expected function name")?;
        let (type_params, type_param_bounds) = self.parse_type_params_with_bounds()?;
        self.expect_symbol(Symbol::LParen, "expected `(` after function name")?;
        let params = self.parse_params()?;
        self.expect_symbol(Symbol::RParen, "expected `)` after parameters")?;
        let (return_type, return_type_span) = if self.consume_symbol(Symbol::Arrow).is_some() {
            let (ty, ty_span) = self.parse_type_annotation("expected return type after `->`")?;
            (Some(ty), Some(ty_span))
        } else {
            (None, None)
        };
        // An extern declaration ends at `;` and has no body.
        let body = if is_extern {
            self.expect_symbol(Symbol::Semicolon, "expected `;` after extern function declaration")?;
            Vec::new()
        } else {
            self.expect_symbol(Symbol::LBrace, "expected function body")?;
            self.parse_block_body()?
        };
        Ok(Function {
            exported,
            reloadable,
            name,
            name_span,
            type_params,
            type_param_bounds,
            params,
            return_type,
            return_type_span,
            body,
            is_extern,
        })
    }

    /// Like [`Self::parse_type_params`] but also parses trait bounds
    /// (`<T: Show, U: Clone + Eq>`). Returns the parameter names and the flat list
    /// of bounds. Used for functions; struct/enum generics stay unbounded.
    fn parse_type_params_with_bounds(
        &mut self,
    ) -> Result<(Vec<String>, Vec<TraitBound>), Diagnostic> {
        if self.consume_symbol(Symbol::Lt).is_none() {
            return Ok((Vec::new(), Vec::new()));
        }
        let mut params = Vec::new();
        let mut bounds = Vec::new();
        loop {
            let (name, name_span) = self.expect_ident_span("expected type parameter name")?;
            if self.consume_symbol(Symbol::Colon).is_some() {
                loop {
                    let (trait_name, trait_name_span) =
                        self.expect_ident_span("expected trait name in bound")?;
                    bounds.push(TraitBound {
                        param: name.clone(),
                        param_span: name_span,
                        trait_name,
                        trait_name_span,
                    });
                    if self.consume_symbol(Symbol::Plus).is_none() {
                        break;
                    }
                }
            }
            params.push(name);
            if self.consume_symbol(Symbol::Comma).is_none() {
                break;
            }
        }
        self.expect_symbol(Symbol::Gt, "expected `>` after type parameters")?;
        Ok((params, bounds))
    }

    /// Parse optional generic type parameters `<T, U>` after a function name.
    /// Returns an empty list when no `<` follows.
    fn parse_type_params(&mut self) -> Result<Vec<String>, Diagnostic> {
        if self.consume_symbol(Symbol::Lt).is_none() {
            return Ok(Vec::new());
        }
        let mut params = Vec::new();
        loop {
            let (name, _) = self.expect_ident_span("expected type parameter name")?;
            params.push(name);
            if self.consume_symbol(Symbol::Comma).is_none() {
                break;
            }
        }
        self.expect_symbol(Symbol::Gt, "expected `>` after type parameters")?;
        Ok(params)
    }

    fn parse_params(&mut self) -> Result<Vec<Param>, Diagnostic> {
        let mut params = Vec::new();
        if self.check_symbol(Symbol::RParen) {
            return Ok(params);
        }
        loop {
            let (name, name_span) = self.expect_ident_span("expected parameter name")?;
            self.expect_symbol(Symbol::Colon, "expected `:` after parameter name")?;
            let (ty, ty_span) = self.parse_type_annotation("expected parameter type")?;
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
                let (ty, ty_span) = self.parse_type_annotation("expected local type")?;
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

        if self.consume_keyword(Keyword::For).is_some() {
            if self.check_symbol(Symbol::LParen) {
                return self.parse_for_c();
            }
            let (binding, binding_span) =
                self.expect_ident_span("expected binding name after `for`")?;
            if self.consume_keyword(Keyword::In).is_none() {
                return Err(self.error_here("PARSE_EXPECTED_IN", "expected `in` after for binding"));
            }
            let mut iterable = self.parse_expr_without_struct_literals()?;
            if self.consume_symbol(Symbol::DotDot).is_some() {
                let end = self.parse_expr_without_struct_literals()?;
                let span = Span::new(iterable.span().start, end.span().end);
                iterable = Expr::Range {
                    start: Box::new(iterable),
                    end: Box::new(end),
                    span,
                };
            }
            self.expect_symbol(Symbol::LBrace, "expected `{` after for iterable")?;
            let body = self.parse_block_body()?;
            return Ok(Stmt::ForIn {
                binding,
                binding_span,
                iterable,
                body,
            });
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
            return Ok(Stmt::Assign {
                target: expr,
                value,
            });
        }
        self.expect_symbol(Symbol::Semicolon, "expected `;` after expression statement")?;
        Ok(Stmt::Expr(expr))
    }

    fn parse_for_c(&mut self) -> Result<Stmt, Diagnostic> {
        self.expect_symbol(Symbol::LParen, "expected `(` after for")?;
        let init = self.parse_stmt()?;
        let condition = self.parse_expr()?;
        self.expect_symbol(Symbol::Semicolon, "expected `;` after for condition")?;
        let step = self.parse_assign_no_semicolon()?;
        self.expect_symbol(Symbol::RParen, "expected `)` after for step")?;
        self.expect_symbol(Symbol::LBrace, "expected `{` after for header")?;
        let body = self.parse_block_body()?;
        Ok(Stmt::ForC {
            init: Box::new(init),
            condition,
            step: Box::new(step),
            body,
        })
    }

    /// Parse an assignment statement without consuming a trailing `;`,
    /// used for the C-style for step which is followed by `)`.
    fn parse_assign_no_semicolon(&mut self) -> Result<Stmt, Diagnostic> {
        let target = self.parse_expr()?;
        if self.consume_symbol(Symbol::Eq).is_some() {
            let value = self.parse_expr()?;
            return Ok(Stmt::Assign { target, value });
        }
        if let Some(op) = self.consume_compound_assign_op() {
            let rhs = self.parse_expr()?;
            let span = Span::new(target.span().start, rhs.span().end);
            let value = Expr::Binary {
                op,
                left: Box::new(target.clone()),
                right: Box::new(rhs),
                span,
            };
            return Ok(Stmt::Assign { target, value });
        }
        Err(self.error_here("PARSE_EXPECTED_ASSIGN", "expected assignment in for step"))
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
        // Optional guard: `pat if <cond> -> ...`. Parsed between the pattern and
        // the arrow so the guard can reference the pattern's bindings.
        let guard = if self.consume_keyword(Keyword::If).is_some() {
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.expect_symbol(Symbol::Arrow, "expected `->` after match pattern")?;
        self.expect_symbol(Symbol::LBrace, "expected `{` after match arm")?;
        let body = self.parse_block_body()?;
        self.consume_symbol(Symbol::Comma);
        Ok(MatchArm { pattern, guard, body })
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
        let mut expr = self.parse_bitwise()?;
        while self.consume_symbol(Symbol::AndAnd).is_some() {
            let right = self.parse_bitwise()?;
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

    fn parse_bitwise(&mut self) -> Result<Expr, Diagnostic> {
        let mut expr = self.parse_comparison()?;
        loop {
            let op = if self.consume_symbol(Symbol::Ampersand).is_some() {
                BinaryOp::BitAnd
            } else if self.consume_symbol(Symbol::Pipe).is_some() {
                BinaryOp::BitOr
            } else if self.consume_symbol(Symbol::Caret).is_some() {
                BinaryOp::BitXor
            } else {
                break;
            };
            let right = self.parse_comparison()?;
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
        if let Some(start) = self.consume_symbol(Symbol::Tilde) {
            let expr = self.parse_unary()?;
            let span = Span::new(start.start, expr.span().end);
            return Ok(Expr::Unary {
                op: UnaryOp::BitNot,
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
                // A numeric field (`t.0`) is a tuple index; otherwise a name.
                // `t.1.0` lexes as a single Float `1.0`, so split it into two
                // consecutive indices (matching how Rust parses tuple chains).
                if let TokenKind::Float(value) = self.peek_kind() {
                    let value = value.clone();
                    let field_span = self.peek().span;
                    self.pos += 1;
                    for part in value.split('.') {
                        let span = Span::new(expr.span().start, field_span.end);
                        expr = Expr::FieldAccess {
                            base: Box::new(expr),
                            field: part.to_string(),
                            field_span,
                            span,
                        };
                    }
                    continue;
                }
                let base_start = expr.span().start;
                // A numeric `.0` is always a tuple index (never a method).
                if let TokenKind::Int(n) = self.peek_kind() {
                    let n = n.clone();
                    let field_span = self.peek().span;
                    self.pos += 1;
                    let span = Span::new(base_start, field_span.end);
                    expr = Expr::FieldAccess {
                        base: Box::new(expr),
                        field: n,
                        field_span,
                        span,
                    };
                    continue;
                }
                let (field, field_span) = self.expect_ident_span("expected field name after `.`")?;
                // Method-call sugar: `recv.method(args)` ≡ `method(recv, args)`,
                // reusing UFCS / trait dispatch (the receiver becomes arg 0). A
                // bare `recv.field` with no `(` stays a field access.
                //
                // Disambiguated from enum/type construction `Type.Variant(..)` by
                // case (the codebase convention; cf. Go/Haskell): an Upper-cased
                // bare-Name receiver is a TYPE, so `Option.Some(7)` stays a
                // qualified construction (handled by the `(` arm via `expr_path`);
                // a lower-cased name or any complex receiver (`xs[1]`, `a.b`) is a
                // value, so its `.m(..)` is a method call.
                if self.check_symbol(Symbol::LParen) && receiver_is_value(&expr) {
                    self.pos += 1; // consume `(`
                    let mut rest = self.parse_call_args()?;
                    let end = self.previous_span().end;
                    let mut args = Vec::with_capacity(rest.len() + 1);
                    args.push(expr);
                    args.append(&mut rest);
                    expr = Expr::Call {
                        callee: field,
                        callee_span: field_span,
                        args,
                        span: Span::new(base_start, end),
                    };
                    continue;
                }
                let span = Span::new(base_start, field_span.end);
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

            if self.consume_symbol(Symbol::Question).is_some() {
                let end = self.previous_span().end;
                let span = Span::new(expr.span().start, end);
                expr = Expr::Try {
                    expr: Box::new(expr),
                    span,
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
            TokenKind::Float(value) => {
                let value = value.clone();
                let span = self.peek().span;
                self.pos += 1;
                Ok(Expr::Float { value, span })
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
                let start = self.peek().span.start;
                self.pos += 1;
                let first = self.parse_expr()?;
                if self.consume_symbol(Symbol::Comma).is_some() {
                    // `(a, b, ...)` is a tuple; a trailing comma `(a,)` is a 1-tuple.
                    let mut elements = vec![first];
                    while !self.check_symbol(Symbol::RParen) {
                        elements.push(self.parse_expr()?);
                        if self.consume_symbol(Symbol::Comma).is_none() {
                            break;
                        }
                    }
                    self.expect_symbol(Symbol::RParen, "expected `)` after tuple")?;
                    let end = self.previous_span().end;
                    Ok(Expr::Tuple {
                        elements,
                        span: Span::new(start, end),
                    })
                } else {
                    self.expect_symbol(Symbol::RParen, "expected `)` after expression")?;
                    Ok(first)
                }
            }
            TokenKind::Symbol(Symbol::LBracket) => self.parse_array_literal(),
            // A `|` (or `||` for zero params) at expression position begins a
            // lambda `|x: Int, y: Int| body`. Binary-or never starts an
            // expression, so this is unambiguous.
            TokenKind::Symbol(Symbol::Pipe) | TokenKind::Symbol(Symbol::OrOr) => {
                self.parse_lambda()
            }
            _ => Err(self.error_here("PARSE_EXPECTED_EXPR", "expected expression")),
        }
    }

    fn parse_lambda(&mut self) -> Result<Expr, Diagnostic> {
        let start = self.peek().span.start;
        // `||` lexes as a single OrOr token: that's an empty parameter list.
        if self.consume_symbol(Symbol::OrOr).is_some() {
            let body = self.parse_expr()?;
            let end = body.span().end;
            return Ok(Expr::Lambda {
                params: Vec::new(),
                body: Box::new(body),
                span: Span::new(start, end),
            });
        }
        self.expect_symbol(Symbol::Pipe, "expected `|` to begin a lambda")?;
        let mut params = Vec::new();
        if !self.check_symbol(Symbol::Pipe) {
            loop {
                let (name, name_span) = self.expect_ident_span("expected lambda parameter name")?;
                // The type annotation is optional: an un-annotated `|x|` parameter
                // has its type inferred from the binding's `fn(T) -> R` annotation
                // (see `desugar::infer_lambda_param_types`). An empty `ty` marks it
                // as not-yet-inferred.
                let (ty, ty_span) = if self.consume_symbol(Symbol::Colon).is_some() {
                    let (ty, ty_span) = self.parse_type_annotation("expected lambda parameter type")?;
                    (ty, Some(ty_span))
                } else {
                    (String::new(), None)
                };
                params.push(Param {
                    name,
                    name_span,
                    ty,
                    ty_span: ty_span.unwrap_or(name_span),
                });
                if self.consume_symbol(Symbol::Comma).is_none() {
                    break;
                }
            }
        }
        self.expect_symbol(Symbol::Pipe, "expected `|` after lambda parameters")?;
        let body = self.parse_expr()?;
        let end = body.span().end;
        Ok(Expr::Lambda {
            params,
            body: Box::new(body),
            span: Span::new(start, end),
        })
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

    /// Parse a type annotation into its canonical type *string*. A leading `(`
    /// introduces a tuple type `(T0, T1, ...)`; a single parenthesized type
    /// `(T)` (no comma) is just grouping and collapses to `T`. Everything else
    /// is a plain identifier (`Int`, `IntArray`, a struct/enum name).
    fn parse_type_annotation(
        &mut self,
        message: &'static str,
    ) -> Result<(String, Span), Diagnostic> {
        // Raw pointer type `*T` (prefix): canonicalized to the type string `*T`.
        // Only valid in type position, so it never clashes with `*` multiply.
        if self.check_symbol(Symbol::Star) {
            let start = self.peek().span.start;
            self.pos += 1;
            let (inner, ispan) = self.parse_type_annotation("expected pointee type after `*`")?;
            return Ok((format!("*{inner}"), Span::new(start, ispan.end)));
        }
        if self.check_keyword(Keyword::Fn) {
            let start = self.peek().span.start;
            self.pos += 1;
            self.expect_symbol(Symbol::LParen, "expected `(` after `fn` in function type")?;
            let mut params = Vec::new();
            if !self.check_symbol(Symbol::RParen) {
                loop {
                    let (ty, _) = self.parse_type_annotation("expected parameter type")?;
                    params.push(ty);
                    if self.consume_symbol(Symbol::Comma).is_none() {
                        break;
                    }
                }
            }
            self.expect_symbol(Symbol::RParen, "expected `)` after function-type parameters")?;
            self.expect_symbol(Symbol::Arrow, "expected `->` in function type")?;
            let (ret, _) = self.parse_type_annotation("expected function return type")?;
            let end = self.previous_span().end;
            let canonical = format!("fn({}) -> {ret}", params.join(", "));
            return Ok((canonical, Span::new(start, end)));
        }
        if self.check_symbol(Symbol::LParen) {
            let start = self.peek().span.start;
            self.pos += 1;
            let mut parts = Vec::new();
            let mut saw_comma = false;
            if !self.check_symbol(Symbol::RParen) {
                loop {
                    let (ty, _) = self.parse_type_annotation("expected type")?;
                    parts.push(ty);
                    if self.consume_symbol(Symbol::Comma).is_some() {
                        saw_comma = true;
                        if self.check_symbol(Symbol::RParen) {
                            break;
                        }
                    } else {
                        break;
                    }
                }
            }
            self.expect_symbol(Symbol::RParen, "expected `)` after tuple type")?;
            let end = self.previous_span().end;
            let span = Span::new(start, end);
            if parts.len() == 1 && !saw_comma {
                return Ok((parts.pop().unwrap(), span));
            }
            return Ok((format!("({})", parts.join(", ")), span));
        }
        let (name, span) = self.expect_ident_span(message)?;
        // Optional generic instantiation args `Name<...>` (e.g. `Box<Int>`,
        // `Result<Int, String>`). The arguments are recursively parsed and kept
        // in the canonical `Name<A0, A1>` string form. The typechecker / MIR
        // verifier strip them back to the base name (instantiation is erased for
        // type checking — the interpreter is the semantic oracle), while native
        // codegen reads them to monomorphize concrete aggregate layouts.
        if self.check_symbol(Symbol::Lt) {
            self.pos += 1;
            let mut args = Vec::new();
            if !self.check_symbol(Symbol::Gt) {
                loop {
                    let (arg, _) = self.parse_type_annotation("expected generic type argument")?;
                    args.push(arg);
                    if self.consume_symbol(Symbol::Comma).is_none() {
                        break;
                    }
                }
            }
            self.expect_symbol(Symbol::Gt, "expected `>` after generic type arguments")?;
            let end = self.previous_span().end;
            let canonical = format!("{name}<{}>", args.join(", "));
            return Ok((canonical, Span::new(span.start, end)));
        }
        Ok((name, span))
    }

    fn expect_symbol(&mut self, symbol: Symbol, message: &'static str) -> Result<(), Diagnostic> {
        if self.consume_symbol(symbol).is_some() {
            Ok(())
        } else {
            Err(self.error_here("PARSE_EXPECTED_SYMBOL", message))
        }
    }

    /// Consume the contextual modifier `name` (e.g. `extern`) ONLY when it
    /// directly precedes `fn`, so it stays usable as an ordinary identifier
    /// elsewhere — same precedent as `reloadable`, no new reserved keyword.
    fn consume_contextual_before_fn(&mut self, name: &str) -> bool {
        let is_ident = matches!(self.peek_kind(), TokenKind::Ident(found) if found == name);
        let followed_by_fn = matches!(
            self.tokens.get(self.pos + 1).map(|token| &token.kind),
            Some(TokenKind::Keyword(Keyword::Fn))
        );
        if is_ident && followed_by_fn {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// Consume a contextual `reloadable` modifier, but ONLY when it directly
    /// precedes `fn` — so `reloadable` stays usable as an ordinary identifier
    /// everywhere else and the lexer needs no new keyword.
    fn consume_reloadable(&mut self) -> bool {
        let is_reloadable_ident =
            matches!(self.peek_kind(), TokenKind::Ident(name) if name == "reloadable");
        let followed_by_fn = matches!(
            self.tokens.get(self.pos + 1).map(|token| &token.kind),
            Some(TokenKind::Keyword(Keyword::Fn))
        );
        if is_reloadable_ident && followed_by_fn {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    /// Consume the current token if it is the identifier `name`. Used for
    /// contextual keywords (`trait`, `impl`) that stay lexically plain
    /// identifiers to avoid perturbing token-kind numbering / self-hosting parity.
    fn consume_contextual(&mut self, name: &str) -> bool {
        if matches!(self.peek_kind(), TokenKind::Ident(found) if found == name) {
            self.pos += 1;
            true
        } else {
            false
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

/// Whether `expr` is an unambiguous VALUE receiver for method-call sugar
/// (`recv.m(..)` → `m(recv, ..)`). A NAME PATH — a bare `Name` or a `.field`
/// chain rooted at one (`a`, `a.b`, `a.b.c`) — is left alone here: it could be a
/// module-qualified call (`demo.math.fn`), enum construction (`Type.Variant`), or
/// a method on a local (`obj.field.m`). That case is disambiguated later by
/// `desugar::desugar_method_calls` (a method only if the path's ROOT is a local).
/// Any OTHER receiver shape (index, call result, parenthesized, ...) cannot be a
/// module/enum prefix, so its `.m(..)` is unambiguously a method call.
fn receiver_is_value(expr: &Expr) -> bool {
    !is_name_path(expr)
}

fn is_name_path(expr: &Expr) -> bool {
    match expr {
        Expr::Name { .. } => true,
        Expr::FieldAccess { base, .. } => is_name_path(base),
        _ => false,
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
