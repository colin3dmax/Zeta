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
        summary: "Stage 0 standard API boundary. The resolver currently accepts std.core and std.io imports.",
        example: "import std.core;",
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
        summary: "Import another module path. Use `as` to create a local alias in module graph programs.",
        example: "import demo.math as math;",
    },
    ReplTopic {
        name: "as",
        summary: "Assign a local alias to an imported module path.",
        example: "import demo.math as math;",
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
        summary: "Branch on a Bool condition. Comparisons and boolean logic return Bool.",
        example: "if ready && !done { return 1; } else { return 0; }",
    },
    ReplTopic {
        name: "while",
        summary: "Loop while a Bool condition is true. Use break or continue for loop-local control flow.",
        example: "while count < 10 { count = count + 1; if count == 3 { continue; } }",
    },
    ReplTopic {
        name: "break",
        summary: "Exit the nearest enclosing while loop.",
        example: "while true { break; }",
    },
    ReplTopic {
        name: "continue",
        summary: "Skip the rest of the current while loop iteration.",
        example: "while count < 3 { count = count + 1; continue; }",
    },
    ReplTopic {
        name: "match",
        summary: "Match a value against simple patterns.",
        example: "match value { 0 -> { return 0; }, _ -> { return value; }, }",
    },
    ReplTopic {
        name: "struct",
        summary: "Declare a record type. Struct literals and field access are executable in full programs.",
        example: "let user: User = User { name: \"Ada\", age: 42 };",
    },
    ReplTopic {
        name: "enum",
        summary: "Declare a tagged set of variants. Use qualified variants such as ResultTag.Ok.",
        example: "let tag: ResultTag = ResultTag.Ok;",
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
        summary: "Boolean scalar type used by if and while conditions. Use &&, ||, and ! to combine Bool values.",
        example: "let ready: Bool = true && !false;",
    },
    ReplTopic {
        name: "IntArray",
        summary: "Homogeneous Int array. Use [..] literals, integer indexing, and .len.",
        example: "let values: IntArray = [2, 4, 6];",
    },
    ReplTopic {
        name: "StringArray",
        summary: "Homogeneous String array. Use [..] literals, integer indexing, and .len.",
        example: "let names: StringArray = [\"Ada\", \"Zeta\"];",
    },
    ReplTopic {
        name: "BoolArray",
        summary: "Homogeneous Bool array. Use [..] literals, integer indexing, and .len.",
        example: "let flags: BoolArray = [true, false];",
    },
    ReplTopic {
        name: "string_len",
        summary: "std.core builtin returning UTF-8 byte length for a String.",
        example: "import std.core; fn main() -> Int { return string_len(\"zeta\"); }",
    },
    ReplTopic {
        name: "string_byte_at",
        summary: "std.core builtin returning the byte at an Int index as Int.",
        example: "import std.core; fn main() -> Int { return string_byte_at(\"A9\", 1); }",
    },
    ReplTopic {
        name: "string_byte_slice",
        summary: "std.core builtin returning a String slice by byte start and byte length.",
        example: "import std.core; fn main() -> String { return string_byte_slice(\"zeta\", 1, 2); }",
    },
    ReplTopic {
        name: "ascii_is_digit",
        summary: "std.core builtin that checks whether an Int byte is ASCII digit.",
        example: "import std.core; fn main() -> Bool { return ascii_is_digit(string_byte_at(\"9\", 0)); }",
    },
    ReplTopic {
        name: "ascii_is_alpha",
        summary: "std.core builtin that checks whether an Int byte is ASCII alphabetic.",
        example: "import std.core; fn main() -> Bool { return ascii_is_alpha(string_byte_at(\"A\", 0)); }",
    },
    ReplTopic {
        name: "ascii_is_alnum",
        summary: "std.core builtin that checks whether an Int byte is ASCII alphabetic or digit.",
        example: "import std.core; fn main() -> Bool { return ascii_is_alnum(string_byte_at(\"A9\", 1)); }",
    },
    ReplTopic {
        name: "ascii_is_whitespace",
        summary: "std.core builtin that checks whether an Int byte is ASCII whitespace.",
        example: "import std.core; fn main() -> Bool { return ascii_is_whitespace(string_byte_at(\" \", 0)); }",
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
    matches.extend(TOPICS.iter().map(|topic| topic.name));
    matches.extend(["std", "std.io", "std.core", "ResultTag.Ok", "ResultTag.Err"]);
    matches.retain(|name| name.starts_with(prefix));
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
        let mut out = String::new();
        out.push_str(&section_title("命令"));
        for (name, summary) in [
            (":help", "查看命令、学习入口和常用主题"),
            (":api", "浏览 Stage 0 API 和标准库边界"),
            (":topics", "列出终端内置文档主题"),
            (":examples", "显示当前阶段可运行示例"),
            (":doc <topic>", "查看某个语言或 API 主题"),
            (":complete <prefix>", "按前缀列出补全候选"),
            (":lang zh|en", "切换本次 REPL 会话语言"),
            (":quit", "退出 REPL"),
        ] {
            out.push_str(&doc_row(
                color(name, Style::Command),
                name,
                &color(summary, Style::Hint),
                22,
            ));
        }
        out.push('\n');
        out.push_str(&section_title("学习入口"));
        for (name, summary) in [
            (":doc getting-started", "从表达式、绑定、函数和运行命令开始"),
            (":doc tutorial", "按阶段学习控制流、数据类型和工具链"),
            (":api", "查看当前可用语言表面和标准库占位"),
        ] {
            out.push_str(&doc_row(
                color(name, Style::Command),
                name,
                &color(summary, Style::Hint),
                22,
            ));
        }
        out.push('\n');
        out.push_str(&section_title("主题索引"));
        out.push_str(&topic_grid(4, 18));
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

fn section_title(title: &str) -> String {
    format!("{}\n", color(format!("◆ {title}"), Style::Bold))
}

fn doc_row(label: String, raw_label: &str, summary: &str, width: usize) -> String {
    let padding = width.saturating_sub(display_width(raw_label));
    format!("  {label}{}{}\n", " ".repeat(padding + 2), summary)
}

fn topic_grid(columns: usize, width: usize) -> String {
    let mut out = String::new();
    for row in TOPICS.chunks(columns) {
        out.push_str("  ");
        for topic in row {
            out.push_str(&color(topic.name, Style::Command));
            let padding = width.saturating_sub(display_width(topic.name));
            out.push_str(&" ".repeat(padding));
        }
        out.push('\n');
    }
    out
}

fn display_width(value: &str) -> usize {
    value
        .chars()
        .map(|ch| if ch.is_ascii() { 1 } else { 2 })
        .sum()
}

pub fn topics_text_colored() -> String {
    topics_text_colored_lang(Language::En)
}

pub fn topics_text_colored_lang(language: Language) -> String {
    let mut out = if language == Language::Zh {
        section_title("文档主题")
    } else {
        String::from("Documentation topics\n")
    };
    for topic in TOPICS {
        out.push_str(&doc_row(
            color(topic.name, Style::Command),
            topic.name,
            &color(topic_summary(topic, language), Style::Hint),
            18,
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
            ("布尔逻辑", "true && !false"),
            ("数组", "fn main() -> Int { let values: IntArray = [2, 4, 6]; return values[0] + values.len; }"),
            ("字符串扫描", "import std.core; fn main() -> Int { return string_len(\"zeta\") + string_byte_at(\"A9\", 1); }"),
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
            ("Boolean", "true && !false"),
            ("Array", "fn main() -> Int { let values: IntArray = [2, 4, 6]; return values[0] + values.len; }"),
            ("String scan", "import std.core; fn main() -> Int { return string_len(\"zeta\") + string_byte_at(\"A9\", 1); }"),
            ("Function", "fn main() -> Int { return 42; }"),
            ("Module", "module demo.core;"),
            ("Doc", ":doc let"),
            ("Completion", ":complete st"),
        ]
    };
    let mut out = if language == Language::Zh {
        section_title("可运行示例")
    } else {
        String::from("Runnable examples\n")
    };
    for (label, example) in examples {
        out.push_str(&doc_row(
            color(label, Style::Bold),
            label,
            &highlight_zeta(example),
            12,
        ));
    }
    out
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
  {}  同质数组，支持字面量、Int 下标和 .len
  {}  标准 API 边界：当前可导入 std.core 和 std.io；std.core 提供字符串 byte 扫描函数

语言表面
  模块/导入、std.core/std.io、函数、绑定/可变绑定、赋值、比较、布尔逻辑、数组字面量/下标/.len、字符串 byte 扫描、返回、if/while、struct、enum 变体、match

试试
  {}
  {}
  {}
",
            color("Int", Style::Type),
            color("String", Style::Type),
            color("Bool", Style::Type),
            color("IntArray/StringArray/BoolArray", Style::Type),
            color("std", Style::Command),
            color(":doc Int", Style::Command),
            color(":doc IntArray", Style::Command),
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
  {}  homogeneous arrays with literals, Int indexing, and .len
  {}  standard API boundary: std.core and std.io imports are accepted; std.core includes string byte scan builtins

Language surface
  module/import, std.core/std.io, fn, let/let mut, assignment, comparison, boolean logic, array literals/index/.len, string byte scan, return, if/else, while, struct, enum variants, match

Try
  {}
  {}
  {}
",
        color("Int", Style::Type),
        color("String", Style::Type),
        color("Bool", Style::Type),
        color("IntArray/StringArray/BoolArray", Style::Type),
        color("std", Style::Command),
        color(":doc Int", Style::Command),
        color(":doc IntArray", Style::Command),
        color(":doc std", Style::Command)
    )
}

pub fn result_line(value: &str) -> String {
    format!(
        "  {} {}\n",
        color("=>", Style::Hint),
        color(value, Style::Value)
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
        "std" => "Stage 0 标准 API 边界；resolver 当前接受 import std.core; 和 import std.io;，未知标准库路径会报错。",
        "playground" => "官网 Playground 运行编译为 WebAssembly 的真实 Zeta 编译器前端。",
        "module" => "声明当前源码模块。",
        "import" => "引入另一个模块路径；module graph 程序中可用 as 创建本地别名。",
        "as" => "为 import 创建本地别名，例如 import demo.math as math;。",
        "fn" => "声明函数。REPL 中语句会被包进临时函数执行。",
        "let" => "声明局部绑定，可以带类型注解；需要重新赋值时使用 let mut。",
        "mut" => "标记局部绑定可变，允许后续赋值语句更新它。",
        "if" => "基于 Bool 条件分支；条件可以使用比较、&&、|| 和 !。",
        "while" => "当 Bool 条件为 true 时循环；条件可以使用比较、&&、|| 和 !，循环内可用 break 或 continue。",
        "break" => "跳出最近一层 while 循环。",
        "continue" => "跳过当前 while 迭代剩余语句，进入下一轮条件检查。",
        "match" => "对标量、枚举变体和通配模式进行分支。",
        "struct" => "声明记录类型；完整程序中可以使用结构体字面量和字段访问。",
        "enum" => "声明标签集合；完整程序中可以使用 ResultTag.Ok 这类限定变体值。",
        "Int" => "当前 Stage 0 checker 支持的整数标量类型。",
        "String" => "当前 Stage 0 checker 支持的字符串标量类型。",
        "Bool" => "if 和 while 条件使用的布尔类型；&&、|| 和 ! 用于组合或取反 Bool。",
        "IntArray" => "同质 Int 数组；支持 [1, 2] 字面量、Int 下标访问和 .len 长度字段。",
        "StringArray" => "同质 String 数组；支持字符串数组字面量、Int 下标访问和 .len 长度字段。",
        "BoolArray" => "同质 Bool 数组；支持布尔数组字面量、Int 下标访问和 .len 长度字段。",
        "string_len" => "std.core 内建函数，返回 String 的 UTF-8 byte 长度。",
        "string_byte_at" => "std.core 内建函数，用 Int 下标读取 String 的单个 byte，并以 Int 返回。",
        "string_byte_slice" => "std.core 内建函数，用 byte 起点和 byte 长度截取 String；切分 UTF-8 字符边界会报运行时错误。",
        "ascii_is_digit" => "std.core 内建函数，判断 Int byte 是否是 ASCII 数字。",
        "ascii_is_alpha" => "std.core 内建函数，判断 Int byte 是否是 ASCII 字母。",
        "ascii_is_alnum" => "std.core 内建函数，判断 Int byte 是否是 ASCII 字母或数字。",
        "ascii_is_whitespace" => "std.core 内建函数，判断 Int byte 是否是 ASCII 空白字符。",
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
            highlight_zeta("true && !false")
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
        highlight_zeta("true && !false")
    )
}

#[derive(Clone, Copy)]
pub enum Style {
    Bold,
    Command,
    Error,
    Keyword,
    Operator,
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
        Style::Operator => "1;38;5;213",
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
        if ch.is_ascii_alphanumeric() || ch == '_' || (ch == ':' && token.is_empty()) {
            token.push(ch);
        } else {
            push_highlighted_token(&mut out, &token);
            token.clear();
            push_highlighted_operator(&mut out, ch);
        }
    }
    push_highlighted_token(&mut out, &token);
    out
}

fn push_highlighted_operator(out: &mut String, ch: char) {
    if matches!(
        ch,
        '=' | '!' | '<' | '>' | '+' | '-' | '*' | '/' | '&' | '|' | ':'
    ) {
        out.push_str(&color(ch.to_string(), Style::Operator));
    } else {
        out.push(ch);
    }
}

fn push_highlighted_token(out: &mut String, token: &str) {
    if token.is_empty() {
        return;
    }
    if matches!(
        token,
        "module"
            | "import"
            | "as"
            | "export"
            | "fn"
            | "let"
            | "mut"
            | "return"
            | "break"
            | "continue"
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
    } else if matches!(token, "true" | "false") {
        out.push_str(&color(token, Style::Value));
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
