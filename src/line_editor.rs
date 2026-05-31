use crate::repl::{color, complete, highlight_zeta, Style};
use std::io::{self, IsTerminal, Read, Write};
use std::process::Command;

const PROMPT: &str = "zeta> ";

pub struct LineEditor {
    history: Vec<String>,
    #[cfg(feature = "repl-rich")]
    rich: Option<RichLineEditor>,
}

impl LineEditor {
    pub fn new() -> Self {
        Self {
            history: Vec::new(),
            #[cfg(feature = "repl-rich")]
            rich: RichLineEditor::new(),
        }
    }

    pub fn read_line(&mut self) -> io::Result<Option<String>> {
        #[cfg(feature = "repl-rich")]
        if let Some(rich) = &mut self.rich {
            match rich.read_line() {
                Ok(line) => return Ok(line),
                Err(_) => {
                    self.rich = None;
                    eprintln!(
                        "reedline unavailable in this terminal; falling back to built-in editor"
                    );
                }
            }
        }

        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return read_plain_line();
        }

        let _raw = RawMode::enable()?;
        let mut input = io::stdin().lock();
        let mut line = String::new();
        let mut cursor = 0usize;
        let mut history_index = self.history.len();
        redraw(&line, cursor)?;

        loop {
            let mut byte = [0u8; 1];
            if input.read(&mut byte)? == 0 {
                return Ok(None);
            }
            match byte[0] {
                b'\r' | b'\n' => {
                    println!();
                    if !line.trim().is_empty() {
                        self.history.push(line.clone());
                    }
                    return Ok(Some(line));
                }
                3 => {
                    println!();
                    return Ok(None);
                }
                4 if line.is_empty() => {
                    println!();
                    return Ok(None);
                }
                9 => {
                    apply_completion(&mut line, &mut cursor)?;
                }
                8 | 127 => {
                    if cursor > 0 {
                        cursor -= 1;
                        line.remove(cursor);
                    }
                }
                27 => {
                    handle_escape(
                        &mut input,
                        &mut line,
                        &mut cursor,
                        &mut history_index,
                        &self.history,
                    )?;
                }
                byte if byte.is_ascii_graphic() || byte == b' ' => {
                    line.insert(cursor, byte as char);
                    cursor += 1;
                }
                _ => {}
            }
            redraw(&line, cursor)?;
        }
    }
}

#[cfg(feature = "repl-rich")]
struct RichLineEditor {
    editor: reedline::Reedline,
    prompt: reedline::DefaultPrompt,
}

#[cfg(feature = "repl-rich")]
impl RichLineEditor {
    fn new() -> Option<Self> {
        if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
            return None;
        }

        use reedline::{
            default_emacs_keybindings, ColumnarMenu, DefaultPrompt, DefaultPromptSegment, Emacs,
            KeyCode, KeyModifiers, MenuBuilder, Reedline, ReedlineEvent, ReedlineMenu,
        };

        let mut keybindings = default_emacs_keybindings();
        keybindings.add_binding(
            KeyModifiers::NONE,
            KeyCode::Tab,
            ReedlineEvent::UntilFound(vec![
                ReedlineEvent::Menu("completion_menu".to_string()),
                ReedlineEvent::MenuNext,
            ]),
        );

        let completion_menu = Box::new(ColumnarMenu::default().with_name("completion_menu"));
        let editor = Reedline::create()
            .with_highlighter(Box::new(ZetaHighlighter))
            .with_completer(Box::new(ZetaCompleter))
            .with_hinter(Box::new(ZetaHinter::default()))
            .with_quick_completions(true)
            .with_partial_completions(true)
            .with_menu(ReedlineMenu::EngineCompleter(completion_menu))
            .with_edit_mode(Box::new(Emacs::new(keybindings)));
        let prompt = DefaultPrompt::new(
            DefaultPromptSegment::Basic(PROMPT.to_string()),
            DefaultPromptSegment::Empty,
        );
        Some(Self { editor, prompt })
    }

    fn read_line(&mut self) -> io::Result<Option<String>> {
        match self.editor.read_line(&self.prompt) {
            Ok(reedline::Signal::Success(line)) => Ok(Some(line)),
            Ok(reedline::Signal::CtrlC | reedline::Signal::CtrlD) => Ok(None),
            Ok(_) => Ok(Some(String::new())),
            Err(err) => Err(io::Error::new(io::ErrorKind::Other, err.to_string())),
        }
    }
}

#[cfg(feature = "repl-rich")]
struct ZetaHighlighter;

#[cfg(feature = "repl-rich")]
impl reedline::Highlighter for ZetaHighlighter {
    fn highlight(&self, line: &str, _cursor: usize) -> reedline::StyledText {
        let mut styled = reedline::StyledText::new();
        for part in zeta_tokens(line) {
            let style = if is_keyword(part.trim) {
                nu_ansi_term::Color::Cyan.bold()
            } else if is_type(part.trim) {
                nu_ansi_term::Color::LightPurple.bold()
            } else if is_command(part.trim) {
                nu_ansi_term::Color::LightYellow.bold()
            } else if is_bool_literal(part.trim) {
                nu_ansi_term::Color::LightGreen.bold()
            } else if is_operator_part(part.text.trim()) {
                nu_ansi_term::Color::LightMagenta.bold()
            } else {
                nu_ansi_term::Style::new()
            };
            styled.push((style, part.text.to_string()));
        }
        styled
    }
}

#[cfg(feature = "repl-rich")]
struct ZetaCompleter;

#[cfg(feature = "repl-rich")]
impl reedline::Completer for ZetaCompleter {
    fn complete(&mut self, line: &str, pos: usize) -> Vec<reedline::Suggestion> {
        let start = token_start(line, pos);
        let prefix = &line[start..pos];
        completion_words()
            .into_iter()
            .filter(|word| word.starts_with(prefix))
            .map(|word| reedline::Suggestion {
                value: word.to_string(),
                span: reedline::Span::new(start, pos),
                append_whitespace: false,
                ..Default::default()
            })
            .collect()
    }
}

#[cfg(feature = "repl-rich")]
#[derive(Default)]
struct ZetaHinter {
    hint: String,
}

#[cfg(feature = "repl-rich")]
impl reedline::Hinter for ZetaHinter {
    fn handle(
        &mut self,
        line: &str,
        _pos: usize,
        _history: &dyn reedline::History,
        _use_ansi_coloring: bool,
        _cwd: &str,
    ) -> String {
        self.hint = plain_hint_for(line).to_string();
        self.hint.clone()
    }

    fn complete_hint(&self) -> String {
        self.hint.clone()
    }

    fn next_hint_token(&self) -> String {
        self.hint
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string()
    }
}

#[cfg(feature = "repl-rich")]
struct TokenPart<'a> {
    text: &'a str,
    trim: &'a str,
}

#[cfg(feature = "repl-rich")]
fn zeta_tokens(line: &str) -> Vec<TokenPart<'_>> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut in_token = false;
    for (index, ch) in line.char_indices() {
        if ch.is_whitespace() {
            if in_token {
                parts.push(token_part(&line[start..index]));
                in_token = false;
            }
            parts.push(token_part(&line[index..index + ch.len_utf8()]));
        } else if !in_token {
            start = index;
            in_token = true;
        }
    }
    if in_token {
        parts.push(token_part(&line[start..]));
    }
    parts
}

#[cfg(feature = "repl-rich")]
fn token_part(text: &str) -> TokenPart<'_> {
    TokenPart {
        text,
        trim: text.trim_matches(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != ':'),
    }
}

#[cfg(feature = "repl-rich")]
fn token_start(line: &str, pos: usize) -> usize {
    line[..pos]
        .rfind(|ch: char| ch.is_whitespace())
        .map(|index| index + 1)
        .unwrap_or(0)
}

#[cfg(feature = "repl-rich")]
fn completion_words() -> Vec<&'static str> {
    let mut words = vec![
        ":help",
        ":doc",
        ":complete",
        ":quit",
        ":exit",
        "std.io",
        "std.core",
    ];
    words.extend(crate::repl::TOPICS.iter().map(|topic| topic.name));
    words
}

#[cfg(feature = "repl-rich")]
fn is_keyword(value: &str) -> bool {
    matches!(
        value,
        "module"
            | "import"
            | "as"
            | "export"
            | "fn"
            | "let"
            | "mut"
            | "return"
            | "if"
            | "else"
            | "while"
            | "match"
            | "struct"
            | "enum"
    )
}

#[cfg(feature = "repl-rich")]
fn is_type(value: &str) -> bool {
    matches!(value, "Int" | "String" | "Bool")
}

#[cfg(feature = "repl-rich")]
fn is_bool_literal(value: &str) -> bool {
    matches!(value, "true" | "false")
}

#[cfg(feature = "repl-rich")]
fn is_operator_part(value: &str) -> bool {
    matches!(
        value,
        "&&" | "||" | "!" | "==" | "!=" | "<" | "<=" | ">" | ">=" | "=" | "+" | "-" | "*" | "/"
    )
}

#[cfg(feature = "repl-rich")]
fn is_command(value: &str) -> bool {
    matches!(value, ":help" | ":doc" | ":complete" | ":quit" | ":exit")
}

fn read_plain_line() -> io::Result<Option<String>> {
    print!("{PROMPT}");
    io::stdout().flush()?;
    let mut line = String::new();
    match io::stdin().read_line(&mut line)? {
        0 => Ok(None),
        _ => Ok(Some(line)),
    }
}

fn handle_escape(
    input: &mut impl Read,
    line: &mut String,
    cursor: &mut usize,
    history_index: &mut usize,
    history: &[String],
) -> io::Result<()> {
    let mut seq = [0u8; 2];
    if input.read(&mut seq)? < 2 || seq[0] != b'[' {
        return Ok(());
    }
    match seq[1] {
        b'D' => {
            *cursor = cursor.saturating_sub(1);
        }
        b'C' => {
            if *cursor < line.len() {
                *cursor += 1;
            }
        }
        b'A' => {
            if !history.is_empty() && *history_index > 0 {
                *history_index -= 1;
                *line = history[*history_index].clone();
                *cursor = line.len();
            }
        }
        b'B' => {
            if *history_index + 1 < history.len() {
                *history_index += 1;
                *line = history[*history_index].clone();
            } else {
                *history_index = history.len();
                line.clear();
            }
            *cursor = line.len();
        }
        _ => {}
    }
    Ok(())
}

fn apply_completion(line: &mut String, cursor: &mut usize) -> io::Result<()> {
    let prefix_start = completion_prefix_start(line, *cursor);
    let prefix = &line[prefix_start..*cursor];
    let matches = complete(prefix);
    if matches.is_empty() {
        return Ok(());
    }
    if matches.len() == 1 {
        let suffix = &matches[0][prefix.len()..];
        line.insert_str(*cursor, suffix);
        *cursor += suffix.len();
    } else {
        println!();
        println!(
            "{}",
            matches
                .into_iter()
                .map(|item| color(item, Style::Command))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
    Ok(())
}

fn completion_prefix_start(line: &str, cursor: usize) -> usize {
    line[..cursor]
        .rfind(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == ':' || ch == '.'))
        .map(|index| index + 1)
        .unwrap_or(0)
}

fn redraw(line: &str, cursor: usize) -> io::Result<()> {
    let hint = hint_for(line);
    print!("\r\x1b[2K{PROMPT}{}{}", highlight_zeta(line), hint);
    let visual_len = line.len() + strip_ansi(&hint).len();
    let back = visual_len.saturating_sub(cursor);
    if back > 0 {
        print!("\x1b[{back}D");
    }
    io::stdout().flush()
}

fn hint_for(line: &str) -> String {
    let hint = plain_hint_for(line);
    if hint.is_empty() {
        String::new()
    } else {
        color(hint, Style::Hint)
    }
}

fn plain_hint_for(line: &str) -> &'static str {
    let trimmed = line.trim_start();
    if trimmed == ":doc" || trimmed == ":doc " {
        return " <topic>";
    }
    if trimmed == ":complete" || trimmed == ":complete " {
        return " <prefix>";
    }
    ""
}

fn strip_ansi(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            for next in chars.by_ref() {
                if next == 'm' {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

struct RawMode {}

impl RawMode {
    fn enable() -> io::Result<Self> {
        let status = Command::new("stty").args(["raw", "-echo"]).status()?;
        if !status.success() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "failed to enter raw terminal mode",
            ));
        }
        Ok(Self {})
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = Command::new("stty").arg("sane").status();
    }
}
