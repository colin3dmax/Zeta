pub struct ReplTopic {
    pub name: &'static str,
    pub summary: &'static str,
    pub example: &'static str,
}

pub const TOPICS: &[ReplTopic] = &[
    ReplTopic {
        name: "module",
        summary: "Declare the current source module.",
        example: "module demo;",
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
        summary: "Declare a local binding with an optional type annotation.",
        example: "let answer: Int = 40 + 2;",
    },
    ReplTopic {
        name: "if",
        summary: "Branch on a Bool condition.",
        example: "if true { return 1; } else { return 0; }",
    },
    ReplTopic {
        name: "while",
        summary: "Loop while a Bool condition is true.",
        example: "while false { let next: Int = 1; }",
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
    TOPICS
        .iter()
        .map(|topic| topic.name)
        .filter(|name| name.starts_with(prefix))
        .collect()
}

pub fn doc(topic: &str) -> Option<&'static ReplTopic> {
    TOPICS.iter().find(|entry| entry.name == topic)
}

pub fn help_text() -> String {
    let topics = TOPICS
        .iter()
        .map(|topic| topic.name)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Commands:\n  :help                 Show this help\n  :doc <topic>          Show short documentation\n  :complete <prefix>    List completions\n  :quit                 Exit\n\nTopics:\n  {topics}\n"
    )
}
