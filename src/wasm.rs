use std::alloc::{alloc, dealloc, Layout};
use std::ptr;
use std::slice;
use std::str;

#[no_mangle]
pub extern "C" fn zeta_alloc(len: usize) -> *mut u8 {
    if len == 0 {
        return ptr::null_mut();
    }
    let layout = match Layout::array::<u8>(len) {
        Ok(layout) => layout,
        Err(_) => return ptr::null_mut(),
    };
    unsafe { alloc(layout) }
}

#[no_mangle]
pub extern "C" fn zeta_dealloc(ptr: *mut u8, len: usize) {
    if ptr.is_null() || len == 0 {
        return;
    }
    if let Ok(layout) = Layout::array::<u8>(len) {
        unsafe {
            dealloc(ptr, layout);
        }
    }
}

#[no_mangle]
pub extern "C" fn zeta_playground(
    mode_ptr: *const u8,
    mode_len: usize,
    source_ptr: *const u8,
    source_len: usize,
) -> u64 {
    let mode = read_utf8(mode_ptr, mode_len);
    let source = read_utf8(source_ptr, source_len);
    let output = match (mode, source) {
        (Ok(mode), Ok(source)) => run_mode(mode, source),
        _ => playground_json(false, "invalid UTF-8 input"),
    };
    leak_result(output)
}

fn read_utf8(ptr: *const u8, len: usize) -> Result<&'static str, ()> {
    if ptr.is_null() {
        return Err(());
    }
    let bytes = unsafe { slice::from_raw_parts(ptr, len) };
    str::from_utf8(bytes).map_err(|_| ())
}

fn run_mode(mode: &str, source: &str) -> String {
    match mode {
        "ast" => match crate::dump_ast(source) {
            Ok(output) => playground_json(true, &output),
            Err(diagnostics) => playground_json(false, &format_diagnostics(&diagnostics)),
        },
        "check" => match crate::check_source(source) {
            Ok(()) => playground_json(true, "ok"),
            Err(diagnostics) => playground_json(false, &format_diagnostics(&diagnostics)),
        },
        "check-module-graph" => match crate::module_graph::check_sources(&virtual_files(source)) {
            Ok(()) => playground_json(true, "ok"),
            Err(errors) => playground_json(false, &format_source_diagnostics(&errors)),
        },
        "run-module-graph" => match crate::module_graph::run_sources(&virtual_files(source)) {
            Ok(value) => playground_json(true, &value.to_string()),
            Err(errors) => playground_json(false, &format_source_diagnostics(&errors)),
        },
        "run" => match crate::run_source(source) {
            Ok(value) => playground_json(true, &value.to_string()),
            Err(diagnostics) => playground_json(false, &format_diagnostics(&diagnostics)),
        },
        _ => playground_json(false, "unknown playground mode"),
    }
}

fn virtual_files(source: &str) -> Vec<crate::module_graph::SourceFile> {
    let mut files = Vec::new();
    let mut current_path = String::from("main.zeta");
    let mut current_source = String::new();
    let mut saw_marker = false;

    for line in source.lines() {
        if let Some(path) = line.trim().strip_prefix("// file: ") {
            if saw_marker || !current_source.trim().is_empty() {
                files.push(crate::module_graph::SourceFile {
                    path: current_path,
                    source: current_source,
                });
            }
            current_path = path.trim().to_string();
            current_source = String::new();
            saw_marker = true;
        } else {
            current_source.push_str(line);
            current_source.push('\n');
        }
    }

    files.push(crate::module_graph::SourceFile {
        path: current_path,
        source: current_source,
    });
    files
}

fn format_diagnostics(diagnostics: &[crate::diagnostic::Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

fn format_source_diagnostics(errors: &[crate::module_graph::SourceDiagnostics]) -> String {
    errors
        .iter()
        .flat_map(|error| {
            error
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.render(&error.source, &error.path))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn playground_json(ok: bool, output: &str) -> String {
    format!(
        "{{\"ok\":{},\"output\":\"{}\"}}",
        if ok { "true" } else { "false" },
        escape_json(output)
    )
}

fn escape_json(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            ch if ch.is_control() => escaped.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => escaped.push(ch),
        }
    }
    escaped
}

fn leak_result(output: String) -> u64 {
    let bytes = output.as_bytes();
    let len = bytes.len();
    let ptr = zeta_alloc(len);
    if ptr.is_null() {
        return 0;
    }
    unsafe {
        ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, len);
    }
    ((ptr as u64) << 32) | len as u64
}
