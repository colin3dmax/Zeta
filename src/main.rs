use std::env;
use std::fs;
use std::io::{self, Write};
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();

    match args.get(1).map(String::as_str) {
        Some("ast-dump") if args.len() == 3 => ast_dump(&args[2]),
        Some("check") if args.len() == 3 => check(&args[2]),
        Some("repl") if args.len() == 2 => repl(),
        _ => {
            eprintln!("usage: zeta ast-dump <path>");
            eprintln!("       zeta check <path>");
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
            for diagnostic in diagnostics {
                eprintln!("{diagnostic}");
            }
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
            for diagnostic in diagnostics {
                eprintln!("{diagnostic}");
            }
            process::exit(1);
        }
    }
}

fn repl() {
    println!("Zeta REPL prototype");
    println!("Type :help for commands. This prototype parses input and prints AST dumps.");

    let stdin = io::stdin();
    let mut line = String::new();
    loop {
        print!("zeta> ");
        io::stdout().flush().expect("stdout should flush");

        line.clear();
        match stdin.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(err) => {
                eprintln!("failed to read input: {err}");
                process::exit(1);
            }
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if matches!(trimmed, ":quit" | ":exit") {
            break;
        }
        if trimmed == ":help" {
            print!("{}", zeta::repl::help_text());
            continue;
        }
        if let Some(topic) = trimmed.strip_prefix(":doc ") {
            match zeta::repl::doc(topic.trim()) {
                Some(entry) => {
                    println!("{}", entry.name);
                    println!("  {}", entry.summary);
                    println!("  example: {}", entry.example);
                }
                None => eprintln!("unknown doc topic `{}`", topic.trim()),
            }
            continue;
        }
        if let Some(prefix) = trimmed.strip_prefix(":complete ") {
            let completions = zeta::repl::complete(prefix.trim());
            if completions.is_empty() {
                println!("no completions");
            } else {
                println!("{}", completions.join(" "));
            }
            continue;
        }

        let source = zeta::repl_source_for_line(trimmed);
        match zeta::dump_ast(&source) {
            Ok(dump) => print!("{dump}"),
            Err(diagnostics) => {
                for diagnostic in diagnostics {
                    eprintln!("{diagnostic}");
                }
            }
        }
    }
}
