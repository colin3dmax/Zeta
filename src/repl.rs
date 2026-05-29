pub struct ReplTopic {
    pub name: &'static str,
    pub summary: &'static str,
    pub example: &'static str,
}

pub struct ReplCommand {
    pub name: &'static str,
    pub summary: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    Zh,
    En,
}

impl Language {
    pub fn code(self) -> &'static str {
        match self {
            Self::Zh => "zh",
            Self::En => "en",
        }
    }
}

pub fn detect_language() -> Language {
    if let Ok(value) = std::env::var("ZETA_LANG") {
        if let Some(language) = parse_language(&value) {
            return language;
        }
    }
    if let Some(language) = config_language() {
        return language;
    }
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(value) = std::env::var(key) {
            if value.to_ascii_lowercase().contains("zh") {
                return Language::Zh;
            }
        }
    }
    Language::En
}

pub fn parse_language(value: &str) -> Option<Language> {
    let normalized = value
        .trim()
        .trim_matches('"')
        .trim_matches('\'')
        .to_ascii_lowercase();
    match normalized.as_str() {
        "zh" | "zh-cn" | "zh_hans" | "zh-hans" | "cn" | "中文" | "简体中文" => {
            Some(Language::Zh)
        }
        "en" | "en-us" | "english" => Some(Language::En),
        _ => None,
    }
}

fn config_language() -> Option<Language> {
    let home = std::env::var("HOME").ok()?;
    let path = std::path::Path::new(&home).join(".zeta/config.toml");
    let config = std::fs::read_to_string(path).ok()?;
    for line in config.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("language") || trimmed.starts_with("lang") {
            if let Some((_, value)) = trimmed.split_once('=') {
                return parse_language(value);
            }
        }
    }
    None
}

pub const COMMANDS: &[ReplCommand] = &[
    ReplCommand {
        name: ":help",
        summary: "Show commands, learning paths, and common topics.",
    },
    ReplCommand {
        name: ":api",
        summary: "Show the Stage 0 API and standard namespace overview.",
    },
    ReplCommand {
        name: ":topics",
        summary: "List all documentation topics available in the terminal.",
    },
    ReplCommand {
        name: ":examples",
        summary: "Show runnable snippets for the current language stage.",
    },
    ReplCommand {
        name: ":doc <topic>",
        summary: "Show short documentation for a language or API topic.",
    },
    ReplCommand {
        name: ":complete <prefix>",
        summary: "List completion candidates for a prefix.",
    },
    ReplCommand {
        name: ":quit",
        summary: "Exit the REPL.",
    },
];

pub const TOPICS: &[ReplTopic] = &[
    ReplTopic {
        name: "getting-started",
        summary: "Start with expressions, let bindings, functions, run/check, and the online playground.",
        example: "40 + 2",
    },
    ReplTopic {
        name: "tutorial",
        summary: "Guided learning path: expressions, functions, control flow, data types, tooling.",
        example: ":doc tutorial",
    },
    ReplTopic {
        name: "api",
        summary: "Browse the Stage 0 built-in language and standard API surface from the terminal.",
        example: ":doc std",
    },
    ReplTopic {
        name: "std",
        summary: "Stage 0 standard library namespace placeholder. Current examples use import std.io; for syntax shaping.",
        example: "import std.io;",
    },
    ReplTopic {
        name: "playground",
        summary: "The website playground runs the real Zeta compiler frontend compiled to WebAssembly.",
        example: "https://zeta.jennieapp.com/#playground",
    },
    ReplTopic {
        name: "module",
        summary: "Declare the current source module.",
        example: "module demo.core;",
    },
    ReplTopic {
        name: "import",
        summary: "Import another module path.",
        example: "import std.io;",
    },
    ReplTopic {
        name: "fn",
        summary: "Declare a function. In the REPL, statements are wrapped in a temporary function.",
        example: "fn main(name: String) -> Int { return 0; }",
    },
    ReplTopic {
        name: "let",
        summary: "Declare a local binding with an optional type annotation. Use `let mut` when the binding must be reassigned.",
        example: "let mut answer: Int = 40; answer = answer + 2;",
    },
    ReplTopic {
        name: "mut",
        summary: "Mark a local binding as mutable so later assignment is allowed.",
        example: "let mut count: Int = 0;",
    },
    ReplTopic {
        name: "if",
        summary: "Branch on a Bool condition.",
        example: "if true { return 1; } else { return 0; }",
    },
    ReplTopic {
        name: "while",
        summary: "Loop while a Bool condition is true.",
        example: "while ready { count = count + 1; }",
    },
    ReplTopic {
        name: "match",
        summary: "Match a value against simple patterns.",
        example: "match value { 0 -> { return 0; }, _ -> { return value; }, }",
    },
    ReplTopic {
        name: "struct",
        summary: "Declare a record type.",
        example: "struct User { name: String, age: Int, }",
    },
    ReplTopic {
        name: "enum",
        summary: "Declare a tagged set of variants.",
        example: "enum ResultTag { Ok, Err, }",
    },
    ReplTopic {
        name: "Int",
        summary: "Integer scalar type currently supported by the Stage 0 checker.",
        example: "let value: Int = 1 + 2;",
    },
    ReplTopic {
        name: "String",
        summary: "String scalar type currently supported by the Stage 0 checker.",
        example: "let name: String = \"zeta\";",
    },
    ReplTopic {
        name: "Bool",
        summary: "Boolean scalar type used by if and while conditions.",
        example: "let ready: Bool = true;",
    },
];

pub fn complete(prefix: &str) -> Vec<&'static str> {
    let mut matches = COMMANDS
        .iter()
        .map(|command| {
            command
                .name
                .split_whitespace()
                .next()
                .unwrap_or(command.name)
        })
        .filter(|name| name.starts_with(prefix))
        .collect::<Vec<_>>();
    matches.extend(
        TOPICS
            .iter()
            .map(|topic| topic.name)
            .filter(|name| name.starts_with(prefix)),
    );
    matches.sort_unstable();
    matches.dedup();
    matches
}

pub fn topic_names() -> String {
    TOPICS
        .iter()
        .map(|topic| topic.name)
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn doc(topic: &str) -> Option<&'static ReplTopic> {
    TOPICS.iter().find(|entry| entry.name == topic)
}

pub fn help_text() -> String {
    let topics = topic_names();
    format!(
        "Commands:\n  :help                 Show this help\n  :api                  Show API overview\n  :topics               List documentation topics\n  :examples             Show runnable examples\n  :doc <topic>          Show short documentation\n  :complete <prefix>    List completions\n  :quit                 Exit\n\nTopics:\n  {topics}\n"
    )
}

pub fn help_text_colored() -> String {
    help_text_colored_lang(Language::En)
}

pub fn help_text_colored_lang(language: Language) -> String {
    if language == Language::Zh {
        let mut out = String::from("命令\n");
        for (name, summary) in [
            (":help", "显示命令、学习路径和常用主题。"),
            (":api", "查看 Stage 0 API 和标准库概览。"),
            (":topics", "列出终端内置文档主题。"),
            (":examples", "显示当前阶段可直接运行的示例。"),
            (":doc <topic>", "查询语言或 API 主题文档。"),
            (":complete <prefix>", "列出补全候选。"),
            (":lang zh|en", "切换本次 REPL 会话语言。"),
            (":quit", "退出 REPL。"),
        ] {
            out.push_str(&format!(
                "  {:<24} {}\n",
                color(name, Style::Command),
                summary
            ));
        }
        out.push_str("\n学习入口\n");
        out.push_str(&format!(
            "  {}\n",
            color(":doc getting-started", Style::Command)
        ));
        out.push_str(&format!("  {}\n", color(":doc tutorial", Style::Command)));
        out.push_str(&format!("  {}\n", color(":api", Style::Command)));
        out.push_str("\n主题\n  ");
        out.push_str(&topic_names());
        out.push('\n');
        return out;
    }

    let mut out = String::from("Commands\n");
    for command in COMMANDS {
        out.push_str(&format!(
            "  {:<20} {}\n",
            color(command.name, Style::Command),
            command.summary
        ));
    }
    out.push_str("\nLearning paths\n");
    out.push_str(&format!(
        "  {}\n",
        color(":doc getting-started", Style::Command)
    ));
    out.push_str(&format!("  {}\n", color(":doc tutorial", Style::Command)));
    out.push_str(&format!("  {}\n", color(":api", Style::Command)));
    out.push_str("\nTopics\n  ");
    out.push_str(&topic_names());
    out.push('\n');
    out
}

pub fn topics_text_colored() -> String {
    topics_text_colored_lang(Language::En)
}

pub fn topics_text_colored_lang(language: Language) -> String {
    let mut out = String::from("Documentation topics\n");
    if language == Language::Zh {
        out = String::from("文档主题\n");
    }
    for topic in TOPICS {
        out.push_str(&format!(
            "  {:<18} {}\n",
            color(topic.name, Style::Command),
            topic_summary(topic, language)
        ));
    }
    out
}

pub fn examples_text_colored() -> String {
    examples_text_colored_lang(Language::En)
}

pub fn examples_text_colored_lang(language: Language) -> String {
    let examples = if language == Language::Zh {
        [
            ("表达式", "40 + 2"),
            ("绑定", "let answer: Int = 40 + 2;"),
            ("可变绑定", "let mut answer: Int = 40;"),
            ("函数", "fn main() -> Int { return 42; }"),
            ("模块", "module demo.core;"),
            ("文档", ":doc let"),
            ("补全", ":complete st"),
        ]
    } else {
        [
            ("Expression", "40 + 2"),
            ("Binding", "let answer: Int = 40 + 2;"),
            ("Mutable", "let mut answer: Int = 40;"),
            ("Function", "fn main() -> Int { return 42; }"),
            ("Module", "module demo.core;"),
            ("Doc", ":doc let"),
            ("Completion", ":complete st"),
        ]
    };
    examples
        .into_iter()
        .map(|(label, example)| format!("  {:<12} {}\n", label, highlight_zeta(example)))
        .collect::<String>()
}

pub fn api_text_colored() -> String {
    api_text_colored_lang(Language::En)
}

pub fn api_text_colored_lang(language: Language) -> String {
    if language == Language::Zh {
        return format!(
            "\
Zeta Stage 0 API
  {}  整数标量和整数算术
  {}  字符串标量
  {}  用于 if/while 条件的布尔标量
  {}  标准库命名空间占位，当前用于 import 示例

语言表面
  module/import, fn, let/let mut, assignment, return, if/else, while, match, struct, enum

试试
  {}
  {}
",
            color("Int", Style::Type),
            color("String", Style::Type),
            color("Bool", Style::Type),
            color("std", Style::Command),
            color(":doc Int", Style::Command),
            color(":doc std", Style::Command)
        );
    }
    api_text_colored_en()
}

fn api_text_colored_en() -> String {
    format!(
        "\
Zeta Stage 0 API
  {}  scalar integer values and arithmetic
  {}  scalar string values
  {}  scalar boolean values for control flow
  {}  namespace placeholder used by import examples

Language surface
  module/import, fn, let/let mut, assignment, return, if/else, while, match, struct, enum

Try
  {}
  {}
",
        color("Int", Style::Type),
        color("String", Style::Type),
        color("Bool", Style::Type),
        color("std", Style::Command),
        color(":doc Int", Style::Command),
        color(":doc std", Style::Command)
    )
}

pub fn doc_text_colored(topic: &ReplTopic, language: Language) -> String {
    format!(
        "{}\n  {}\n  {}: {}\n",
        color(topic.name, Style::Command),
        topic_summary(topic, language),
        if language == Language::Zh {
            "示例"
        } else {
            "example"
        },
        highlight_zeta(topic.example)
    )
}

fn topic_summary(topic: &ReplTopic, language: Language) -> &'static str {
    if language == Language::En {
        return topic.summary;
    }
    match topic.name {
        "getting-started" => "从表达式、let/let mut 绑定、函数、run/check 和在线 Playground 开始。",
        "tutorial" => "推荐学习路径：表达式、函数、控制流、数据类型和工具链。",
        "api" => "在终端里浏览 Stage 0 内置语言能力和标准 API 表面。",
        "std" => "Stage 0 标准库命名空间占位。当前示例用 import std.io; 塑造语法。",
        "playground" => "官网 Playground 运行编译为 WebAssembly 的真实 Zeta 编译器前端。",
        "module" => "声明当前源码模块。",
        "import" => "引入另一个模块路径。",
        "fn" => "声明函数。REPL 中语句会被包进临时函数执行。",
        "let" => "声明局部绑定，可以带类型注解；需要重新赋值时使用 let mut。",
        "mut" => "标记局部绑定可变，允许后续赋值语句更新它。",
        "if" => "基于 Bool 条件分支。",
        "while" => "当 Bool 条件为 true 时循环。",
        "match" => "对值进行简单模式匹配。",
        "struct" => "声明记录类型。",
        "enum" => "声明标签集合。",
        "Int" => "当前 Stage 0 checker 支持的整数标量类型。",
        "String" => "当前 Stage 0 checker 支持的字符串标量类型。",
        "Bool" => "if 和 while 条件使用的布尔类型。",
        _ => topic.summary,
    }
}

pub fn welcome_banner() -> String {
    welcome_banner_lang(Language::En)
}

pub fn welcome_banner_lang(language: Language) -> String {
    if language == Language::Zh {
        return format!(
            "\
\x1b[1mZETA\x1b[0m  \x1b[2mStage 0 语言交互终端 | 真实编译器执行 | 终端内置文档\x1b[0m
{}  查看命令  {}  API 概览  {}  示例  {}  切换英文
试试: {}    {}
",
            color(":help", Style::Command),
            color(":api", Style::Command),
            color(":examples", Style::Command),
            color(":lang en", Style::Command),
            highlight_zeta("40 + 2"),
            highlight_zeta("let mut answer: Int = 40;")
        );
    }

    format!(
        "\
\x1b[1mZETA\x1b[0m  \x1b[2mStage 0 language shell | compiler-backed execution | docs in terminal\x1b[0m
{}  help  {}  API  {}  examples  {}  switch Chinese
Try: {}    {}
",
        color(":help", Style::Command),
        color(":api", Style::Command),
        color(":examples", Style::Command),
        color(":lang zh", Style::Command),
        highlight_zeta("40 + 2"),
        highlight_zeta("let mut answer: Int = 40;")
    )
}

#[derive(Clone, Copy)]
pub enum Style {
    Bold,
    Command,
    Error,
    Keyword,
    Type,
    Value,
    Hint,
}

pub fn color(text: impl AsRef<str>, style: Style) -> String {
    let code = match style {
        Style::Bold => "1",
        Style::Command => "1;38;5;214",
        Style::Error => "1;38;5;196",
        Style::Keyword => "1;38;5;81",
        Style::Type => "1;38;5;141",
        Style::Value => "1;38;5;120",
        Style::Hint => "38;5;245",
    };
    format!("\x1b[{code}m{}\x1b[0m", text.as_ref())
}

pub fn highlight_zeta(source: &str) -> String {
    let mut out = String::new();
    let mut token = String::new();
    for ch in source.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == ':' {
            token.push(ch);
        } else {
            push_highlighted_token(&mut out, &token);
            token.clear();
            out.push(ch);
        }
    }
    push_highlighted_token(&mut out, &token);
    out
}

fn push_highlighted_token(out: &mut String, token: &str) {
    if token.is_empty() {
        return;
    }
    if matches!(
        token,
        "module"
            | "import"
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
    ) {
        out.push_str(&color(token, Style::Keyword));
    } else if matches!(token, "Int" | "String" | "Bool") {
        out.push_str(&color(token, Style::Type));
    } else if matches!(
        token,
        ":help"
            | ":api"
            | ":topics"
            | ":examples"
            | ":doc"
            | ":complete"
            | ":lang"
            | ":quit"
            | ":exit"
    ) {
        out.push_str(&color(token, Style::Command));
    } else {
        out.push_str(token);
    }
}
