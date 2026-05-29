use crate::diagnostic::{Diagnostic, Span};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    Ident(String),
    Int(String),
    String(String),
    Keyword(Keyword),
    Symbol(Symbol),
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    Module,
    Import,
    Export,
    Fn,
    Let,
    Mut,
    Return,
    If,
    Else,
    While,
    Match,
    Struct,
    Enum,
    True,
    False,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Symbol {
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Colon,
    Semicolon,
    Comma,
    Dot,
    Arrow,
    Eq,
    EqEq,
    BangEq,
    Lt,
    Lte,
    Gt,
    Gte,
    Plus,
    Minus,
    Star,
    Slash,
}

pub fn lex(source: &str) -> Result<Vec<Token>, Vec<Diagnostic>> {
    Lexer::new(source).lex()
}

struct Lexer<'a> {
    source: &'a str,
    pos: usize,
    diagnostics: Vec<Diagnostic>,
    tokens: Vec<Token>,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            pos: 0,
            diagnostics: Vec::new(),
            tokens: Vec::new(),
        }
    }

    fn lex(mut self) -> Result<Vec<Token>, Vec<Diagnostic>> {
        while let Some(ch) = self.peek_char() {
            match ch {
                c if c.is_whitespace() => {
                    self.bump_char();
                }
                '/' if self.peek_next_char() == Some('/') => self.skip_line_comment(),
                c if is_ident_start(c) => self.lex_ident_or_keyword(),
                c if c.is_ascii_digit() => self.lex_int(),
                '"' => self.lex_string(),
                _ => self.lex_symbol(),
            }
        }

        self.tokens.push(Token {
            kind: TokenKind::Eof,
            span: Span::new(self.pos, self.pos),
        });

        if self.diagnostics.is_empty() {
            Ok(self.tokens)
        } else {
            Err(self.diagnostics)
        }
    }

    fn lex_ident_or_keyword(&mut self) {
        let start = self.pos;
        self.bump_char();
        while matches!(self.peek_char(), Some(c) if is_ident_continue(c)) {
            self.bump_char();
        }
        let text = &self.source[start..self.pos];
        let kind = match text {
            "module" => TokenKind::Keyword(Keyword::Module),
            "import" => TokenKind::Keyword(Keyword::Import),
            "export" => TokenKind::Keyword(Keyword::Export),
            "fn" => TokenKind::Keyword(Keyword::Fn),
            "let" => TokenKind::Keyword(Keyword::Let),
            "mut" => TokenKind::Keyword(Keyword::Mut),
            "return" => TokenKind::Keyword(Keyword::Return),
            "if" => TokenKind::Keyword(Keyword::If),
            "else" => TokenKind::Keyword(Keyword::Else),
            "while" => TokenKind::Keyword(Keyword::While),
            "match" => TokenKind::Keyword(Keyword::Match),
            "struct" => TokenKind::Keyword(Keyword::Struct),
            "enum" => TokenKind::Keyword(Keyword::Enum),
            "true" => TokenKind::Keyword(Keyword::True),
            "false" => TokenKind::Keyword(Keyword::False),
            _ => TokenKind::Ident(text.to_string()),
        };
        self.tokens.push(Token {
            kind,
            span: Span::new(start, self.pos),
        });
    }

    fn lex_int(&mut self) {
        let start = self.pos;
        self.bump_char();
        while matches!(self.peek_char(), Some(c) if c.is_ascii_digit()) {
            self.bump_char();
        }
        if self.peek_char() == Some('.') {
            self.bump_char();
            while matches!(self.peek_char(), Some(c) if c.is_ascii_digit()) {
                self.bump_char();
            }
            self.diagnostics.push(Diagnostic::new(
                "LEX_FLOAT_UNSUPPORTED",
                "floating-point literals are not supported in Stage 0; use Int arithmetic for now",
                Span::new(start, self.pos),
            ));
            return;
        }
        self.tokens.push(Token {
            kind: TokenKind::Int(self.source[start..self.pos].to_string()),
            span: Span::new(start, self.pos),
        });
    }

    fn lex_string(&mut self) {
        let start = self.pos;
        self.bump_char();
        let mut value = String::new();
        while let Some(ch) = self.peek_char() {
            match ch {
                '"' => {
                    self.bump_char();
                    self.tokens.push(Token {
                        kind: TokenKind::String(value),
                        span: Span::new(start, self.pos),
                    });
                    return;
                }
                '\\' => {
                    self.bump_char();
                    match self.peek_char() {
                        Some('"') => {
                            value.push('"');
                            self.bump_char();
                        }
                        Some('n') => {
                            value.push('\n');
                            self.bump_char();
                        }
                        Some(other) => {
                            value.push(other);
                            self.bump_char();
                        }
                        None => break,
                    }
                }
                other => {
                    value.push(other);
                    self.bump_char();
                }
            }
        }
        self.diagnostics.push(Diagnostic::new(
            "LEX_UNTERMINATED_STRING",
            "unterminated string literal",
            Span::new(start, self.pos),
        ));
    }

    fn lex_symbol(&mut self) {
        let start = self.pos;
        let Some(ch) = self.bump_char() else {
            return;
        };
        let kind = match ch {
            '(' => Some(TokenKind::Symbol(Symbol::LParen)),
            ')' => Some(TokenKind::Symbol(Symbol::RParen)),
            '{' => Some(TokenKind::Symbol(Symbol::LBrace)),
            '}' => Some(TokenKind::Symbol(Symbol::RBrace)),
            '[' => Some(TokenKind::Symbol(Symbol::LBracket)),
            ']' => Some(TokenKind::Symbol(Symbol::RBracket)),
            ':' => Some(TokenKind::Symbol(Symbol::Colon)),
            ';' => Some(TokenKind::Symbol(Symbol::Semicolon)),
            ',' => Some(TokenKind::Symbol(Symbol::Comma)),
            '.' => Some(TokenKind::Symbol(Symbol::Dot)),
            '=' if self.peek_char() == Some('=') => {
                self.bump_char();
                Some(TokenKind::Symbol(Symbol::EqEq))
            }
            '=' => Some(TokenKind::Symbol(Symbol::Eq)),
            '!' if self.peek_char() == Some('=') => {
                self.bump_char();
                Some(TokenKind::Symbol(Symbol::BangEq))
            }
            '<' if self.peek_char() == Some('=') => {
                self.bump_char();
                Some(TokenKind::Symbol(Symbol::Lte))
            }
            '<' => Some(TokenKind::Symbol(Symbol::Lt)),
            '>' if self.peek_char() == Some('=') => {
                self.bump_char();
                Some(TokenKind::Symbol(Symbol::Gte))
            }
            '>' => Some(TokenKind::Symbol(Symbol::Gt)),
            '+' => Some(TokenKind::Symbol(Symbol::Plus)),
            '*' => Some(TokenKind::Symbol(Symbol::Star)),
            '/' => Some(TokenKind::Symbol(Symbol::Slash)),
            '-' if self.peek_char() == Some('>') => {
                self.bump_char();
                Some(TokenKind::Symbol(Symbol::Arrow))
            }
            '-' => Some(TokenKind::Symbol(Symbol::Minus)),
            _ => None,
        };

        if let Some(kind) = kind {
            self.tokens.push(Token {
                kind,
                span: Span::new(start, self.pos),
            });
        } else {
            self.diagnostics.push(Diagnostic::new(
                "LEX_UNKNOWN_CHAR",
                format!("unknown character `{ch}`"),
                Span::new(start, self.pos),
            ));
        }
    }

    fn skip_line_comment(&mut self) {
        while let Some(ch) = self.peek_char() {
            self.bump_char();
            if ch == '\n' {
                break;
            }
        }
    }

    fn peek_char(&self) -> Option<char> {
        self.source[self.pos..].chars().next()
    }

    fn peek_next_char(&self) -> Option<char> {
        let mut chars = self.source[self.pos..].chars();
        chars.next()?;
        chars.next()
    }

    fn bump_char(&mut self) -> Option<char> {
        let ch = self.peek_char()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }
}

fn is_ident_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_ident_continue(ch: char) -> bool {
    is_ident_start(ch) || ch.is_ascii_digit()
}
