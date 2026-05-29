use std::env;
use std::fs;
use std::process;
use zeta::repl::{color, Language, Style};

fn main() {
    let args: Vec<String> = env::args().collect();

    match args.get(1).map(String::as_str) {
        Some("ast-dump") if args.len() == 3 => ast_dump(&args[2]),
        Some("hir-dump") if args.len() == 3 => hir_dump(&args[2]),
        Some("mir-dump") if args.len() == 3 => mir_dump(&args[2]),
        Some("check") if args.len() == 3 => check(&args[2]),
        Some("run") if args.len() == 3 => run(&args[2]),
        Some("repl") if args.len() == 2 => repl(),
        _ => {
            eprintln!("usage: zeta ast-dump <path>");
            eprintln!("       zeta hir-dump <path>");
            eprintln!("       zeta mir-dump <path>");
            eprintln!("       zeta check <path>");
            eprintln!("       zeta run <path>");
            eprintln!("       zeta repl");
            process::exit(2);
        }
    }
}

fn ast_dump(path: &str) {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(err) => {
            eprintln!("failed to read {path}: {err}");
            process::exit(1);
        }
    };

    match zeta::dump_ast(&source) {
        Ok(dump) => print!("{dump}"),
        Err(diagnostics) => {
            print_diagnostics(&diagnostics, &source, path);
            process::exit(1);
        }
    }
}

fn hir_dump(path: &str) {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(err) => {
            eprintln!("failed to read {path}: {err}");
            process::exit(1);
        }
    };

    match zeta::dump_hir(&source) {
        Ok(dump) => print!("{dump}"),
        Err(diagnostics) => {
            print_diagnostics(&diagnostics, &source, path);
            process::exit(1);
        }
    }
}

fn mir_dump(path: &str) {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(err) => {
            eprintln!("failed to read {path}: {err}");
            process::exit(1);
        }
    };

    match zeta::dump_mir(&source) {
        Ok(dump) => print!("{dump}"),
        Err(diagnostics) => {
            print_diagnostics(&diagnostics, &source, path);
            process::exit(1);
        }
    }
}

fn check(path: &str) {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(err) => {
            eprintln!("failed to read {path}: {err}");
            process::exit(1);
        }
    };

    match zeta::check_source(&source) {
        Ok(()) => println!("ok"),
        Err(diagnostics) => {
            print_diagnostics(&diagnostics, &source, path);
            process::exit(1);
        }
    }
}

fn run(path: &str) {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(err) => {
            eprintln!("failed to read {path}: {err}");
            process::exit(1);
        }
    };

    match zeta::run_source(&source) {
        Ok(value) => println!("{value}"),
        Err(diagnostics) => {
            print_diagnostics(&diagnostics, &source, path);
            process::exit(1);
        }
    }
}

fn print_diagnostics(diagnostics: &[zeta::diagnostic::Diagnostic], source: &str, path: &str) {
    for diagnostic in diagnostics {
        eprintln!("{}", diagnostic.render(source, path));
    }
}

fn repl() {
    let mut language = zeta::repl::detect_language();
    print!("{}", zeta::repl::welcome_banner_lang(language));

    let mut editor = zeta::line_editor::LineEditor::new();
    let mut session = zeta::runtime::ReplSession::new();
    loop {
        let line = match editor.read_line() {
            Ok(Some(line)) => line,
            Ok(None) => break,
            Err(err) => {
                eprintln!("failed to read input: {err}");
                process::exit(1);
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if matches!(trimmed, ":quit" | ":exit") {
            break;
        }
        if trimmed == ":help" {
            print!("{}", zeta::repl::help_text_colored_lang(language));
            continue;
        }
        if trimmed == ":api" {
            print!("{}", zeta::repl::api_text_colored_lang(language));
            continue;
        }
        if trimmed == ":topics" {
            print!("{}", zeta::repl::topics_text_colored_lang(language));
            continue;
        }
        if trimmed == ":examples" {
            print!("{}", zeta::repl::examples_text_colored_lang(language));
            continue;
        }
        if trimmed == ":lang" {
            println!("usage: {}", color(":lang zh|en", Style::Command));
            continue;
        }
        if let Some(value) = trimmed.strip_prefix(":lang ") {
            match zeta::repl::parse_language(value) {
                Some(next) => {
                    language = next;
                    let message = if language == Language::Zh {
                        "已切换到中文。"
                    } else {
                        "Switched to English."
                    };
                    println!("{}", color(message, Style::Value));
                }
                None => eprintln!("unknown language `{}`", value.trim()),
            }
            continue;
        }
        if trimmed == ":doc" {
            println!("usage: {}", color(":doc <topic>", Style::Command));
            println!("topics: {}", zeta::repl::topic_names());
            continue;
        }
        if let Some(topic) = trimmed.strip_prefix(":doc ") {
            match zeta::repl::doc(topic.trim()) {
                Some(entry) => {
                    print!("{}", zeta::repl::doc_text_colored(entry, language));
                }
                None => eprintln!("unknown doc topic `{}`", topic.trim()),
            }
            continue;
        }
        if trimmed == ":complete" {
            println!("usage: {}", color(":complete <prefix>", Style::Command));
            continue;
        }
        if let Some(prefix) = trimmed.strip_prefix(":complete ") {
            let completions = zeta::repl::complete(prefix.trim());
            if completions.is_empty() {
                println!("no completions");
            } else {
                println!(
                    "{}",
                    completions
                        .into_iter()
                        .map(|item| color(item, Style::Command))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
            }
            continue;
        }

        let source = zeta::repl_run_source_for_line(trimmed)
            .unwrap_or_else(|| zeta::repl_source_for_line(trimmed));
        match zeta::eval_repl_source(&mut session, &source) {
            Ok(value) => {
                if value != zeta::runtime::Value::Unit {
                    println!("{}", color(value.to_string(), Style::Value));
                } else {
                    println!("{}", color("ok", Style::Value));
                }
            }
            Err(diagnostics) => {
                for diagnostic in diagnostics {
                    print_repl_diagnostic(&diagnostic);
                }
            }
        }
    }
}

fn print_repl_diagnostic(diagnostic: &zeta::diagnostic::Diagnostic) {
    eprintln!(
        "{}: {}",
        color(diagnostic.code, Style::Error),
        diagnostic.message
    );
}
