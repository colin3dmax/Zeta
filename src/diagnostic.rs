use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
    pub span: Span,
}

impl Diagnostic {
    pub fn new(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        Self {
            code,
            message: message.into(),
            span,
        }
    }

    pub fn render(&self, source: &str, path: &str) -> String {
        let location = SourceLocation::new(source, self.span);
        format!(
            "{code} at {path}:{line}:{column}: {message}\n  |\n{line:>2} | {text}\n  | {marker}",
            code = self.code,
            line = location.line,
            column = location.column,
            message = self.message,
            text = location.line_text,
            marker = location.marker,
        )
    }
}

struct SourceLocation {
    line: usize,
    column: usize,
    line_text: String,
    marker: String,
}

impl SourceLocation {
    fn new(source: &str, span: Span) -> Self {
        let start = span.start.min(source.len());
        let end = span.end.min(source.len()).max(start);
        let line_start = source[..start]
            .rfind('\n')
            .map(|index| index + 1)
            .unwrap_or(0);
        let line_end = source[start..]
            .find('\n')
            .map(|index| start + index)
            .unwrap_or(source.len());
        let line = source[..line_start]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1;
        let column = source[line_start..start].chars().count() + 1;
        let width = source[start..end].chars().count().max(1);
        let marker = format!(
            "{}{}",
            " ".repeat(column.saturating_sub(1)),
            "^".repeat(width)
        );
        Self {
            line,
            column,
            line_text: source[line_start..line_end].to_string(),
            marker,
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at {}..{}: {}",
            self.code, self.span.start, self.span.end, self.message
        )
    }
}
