use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process;
use zeta::repl::{color, Language, Style};

fn main() {
    let args: Vec<String> = env::args().collect();

    match args.get(1).map(String::as_str) {
        Some("ast-dump") if args.len() == 3 => ast_dump(&args[2]),
        Some("hir-dump") if args.len() == 3 => hir_dump(&args[2]),
        Some("mir-dump") if args.len() == 3 => mir_dump(&args[2]),
        Some("symbols-dump") if args.len() == 3 => symbols_dump(&args[2]),
        Some("check") if args.len() == 3 => check(&args[2]),
        Some("emit-ir") if args.len() == 3 => emit_ir(&args[2]),
        Some("run") if args.len() == 3 => run(&args[2]),
        Some("serve") if args.len() == 3 => serve(&args[2]),
        Some("repl") if args.len() == 2 => repl(),
        _ => {
            eprintln!("usage: zeta ast-dump <path>");
            eprintln!("       zeta hir-dump <path>");
            eprintln!("       zeta mir-dump <path>");
            eprintln!("       zeta symbols-dump <directory>");
            eprintln!("       zeta check <path>");
            eprintln!("       zeta emit-ir <path>   (requires --features llvm)");
            eprintln!("       zeta run <path>");
            eprintln!("       zeta serve <path>");
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

fn symbols_dump(path: &str) {
    let path_ref = Path::new(path);
    if !path_ref.is_dir() {
        eprintln!("symbols-dump expects a module directory");
        process::exit(1);
    }
    let files = load_module_directory(path_ref);
    match zeta::module_graph::dump_symbols(&files) {
        Ok(dump) => print!("{dump}"),
        Err(errors) => {
            for error in errors {
                print_diagnostics(&error.diagnostics, &error.source, &error.path);
            }
            process::exit(1);
        }
    }
}

fn check(path: &str) {
    let path_ref = Path::new(path);
    if path_ref.is_dir() {
        check_module_directory(path_ref);
        return;
    }

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

fn check_module_directory(root: &Path) {
    let files = load_module_directory(root);

    match zeta::module_graph::check_sources(&files) {
        Ok(()) => println!("ok"),
        Err(errors) => {
            for error in errors {
                print_diagnostics(&error.diagnostics, &error.source, &error.path);
            }
            process::exit(1);
        }
    }
}

fn zeta_files(root: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    collect_zeta_files(root, &mut out)?;
    out.sort();
    Ok(out)
}

fn collect_zeta_files(path: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_zeta_files(&path, out)?;
        } else if path.extension().and_then(|value| value.to_str()) == Some("zeta") {
            out.push(path);
        }
    }
    Ok(())
}

fn load_module_directory(root: &Path) -> Vec<zeta::module_graph::SourceFile> {
    let paths = match zeta_files(root) {
        Ok(paths) => paths,
        Err(err) => {
            eprintln!("failed to scan {}: {err}", root.display());
            process::exit(1);
        }
    };
    if paths.is_empty() {
        eprintln!("no .zeta files found in {}", root.display());
        process::exit(1);
    }

    let mut files = Vec::new();
    for path in paths {
        let source = match fs::read_to_string(&path) {
            Ok(source) => source,
            Err(err) => {
                eprintln!("failed to read {}: {err}", path.display());
                process::exit(1);
            }
        };
        files.push(zeta::module_graph::SourceFile {
            path: path.display().to_string(),
            source,
        });
    }
    files
}

/// `zeta emit-ir <file>` — lower a single source to textual LLVM IR (the same IR
/// the native backend JIT/AOT consumes). The freestanding kernel build pipes
/// this through `clang --target=riscv64` (see `kernel/`). Requires `--features
/// llvm`.
#[cfg(feature = "llvm")]
fn emit_ir(path: &str) {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(err) => {
            eprintln!("failed to read {path}: {err}");
            process::exit(1);
        }
    };
    let module = match zeta::parse_source(&source) {
        Ok(module) => module,
        Err(diagnostics) => {
            print_diagnostics(&diagnostics, &source, path);
            process::exit(1);
        }
    };
    let structs: Vec<zeta::ast::StructDecl> = module
        .items
        .iter()
        .filter_map(|item| match item {
            zeta::ast::Item::Struct(decl) => Some(decl.clone()),
            _ => None,
        })
        .collect();
    let program = match zeta::lower_source(&source) {
        Ok(program) => program,
        Err(diagnostics) => {
            print_diagnostics(&diagnostics, &source, path);
            process::exit(1);
        }
    };
    match zeta::codegen::emit_llvm_ir(&program, &structs) {
        Ok(ir) => print!("{ir}"),
        Err(err) => {
            eprintln!("codegen error: {err}");
            process::exit(1);
        }
    }
}

#[cfg(not(feature = "llvm"))]
fn emit_ir(_path: &str) {
    eprintln!("emit-ir requires building with `--features llvm`");
    process::exit(2);
}

fn run(path: &str) {
    let path_ref = Path::new(path);
    if path_ref.is_dir() {
        run_module_directory(path_ref);
        return;
    }

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

/// `zeta serve <file>` — run a hot-reloadable service (the `init`/`step`/`render`
/// convention; see docs/compiler/hot-reload-design.md). Prints the rendered
/// state, reads integer inputs (one per line) and ticks the service. While it
/// waits for input you can edit & save the file; the next input hot-reloads the
/// new code WITHOUT losing the accumulated state. A bad edit is rejected and the
/// previous version keeps running.
fn serve(path: &str) {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(err) => {
            eprintln!("failed to read {path}: {err}");
            process::exit(1);
        }
    };
    let mut driver = match zeta::runtime::ServiceDriver::start(&source) {
        Ok(driver) => driver,
        Err(diagnostics) => {
            print_diagnostics(&diagnostics, &source, path);
            process::exit(1);
        }
    };

    let mut last_mtime = file_mtime(path);
    eprintln!(
        "[zeta serve] {path} — integer inputs, one per line; \
         edit & save the file to hot-reload; Ctrl-D to quit."
    );

    loop {
        match driver.render() {
            Ok(text) => println!("{text}"),
            Err(diagnostics) => print_diagnostics(&diagnostics, &source, path),
        }

        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) => break, // EOF (Ctrl-D)
            Ok(_) => {}
            Err(err) => {
                eprintln!("input error: {err}");
                break;
            }
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Hot-reload if the file changed since the last tick.
        let now_mtime = file_mtime(path);
        if now_mtime != last_mtime {
            last_mtime = now_mtime;
            match fs::read_to_string(path) {
                Ok(new_source) => match driver.try_reload(&new_source) {
                    Ok(()) => eprintln!("[reloaded {path}]"),
                    Err(diagnostics) => {
                        eprintln!("[reload rejected — keeping the previous version]");
                        print_diagnostics(&diagnostics, &new_source, path);
                    }
                },
                Err(err) => eprintln!("[reload read failed: {err}]"),
            }
        }

        match line.parse::<i64>() {
            Ok(input) => {
                if let Err(diagnostics) = driver.tick(zeta::runtime::Value::Int(input)) {
                    print_diagnostics(&diagnostics, &source, path);
                }
            }
            Err(_) => eprintln!("[expected an integer input, got `{line}`]"),
        }
    }
}

fn file_mtime(path: &str) -> Option<std::time::SystemTime> {
    fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

fn run_module_directory(root: &Path) {
    let files = load_module_directory(root);

    match zeta::module_graph::run_sources(&files) {
        Ok(value) => println!("{value}"),
        Err(errors) => {
            for error in errors {
                print_diagnostics(&error.diagnostics, &error.source, &error.path);
            }
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
                    print!("{}", zeta::repl::result_line(&value.to_string()));
                } else {
                    print!("{}", zeta::repl::result_line("ok"));
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
