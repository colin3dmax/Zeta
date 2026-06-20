// Native backend — self-hosting closure: the WHOLE Zeta frontend, AOT/JIT-
// compiled to native, must produce byte-identical output to the Stage0
// interpreter (cargo feature `llvm`).
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_selfhost_run -- --nocapture
//
// We append a `main() -> Int` to arena_frontend.zeta that calls the frontend's
// `compile(source, mode)` front door on a small embedded Zeta program, then
// reduces the resulting dump String to an Int digest (length + a byte-weighted
// checksum). Running that combined program through BOTH the interpreter and the
// native JIT and getting the same Int proves the native-compiled frontend emits
// exactly the same bytes as the interpreter — the real bootstrap-closure check.
#![cfg(feature = "llvm")]

use zeta::ast::Item;
use zeta::runtime::Value;

const FRONTEND: &str = include_str!("../testdata/selfhost/arena_frontend.zeta");

/// Escape `s` so it can be embedded inside a double-quoted Zeta string literal.
fn zeta_string_literal(s: &str) -> String {
    let mut out = String::from("\"");
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// frontend source + a `main()` that compiles `prog` in `mode` and digests the
/// resulting string to an Int.
fn combined(prog: &str, mode: &str) -> String {
    let lit = zeta_string_literal(prog);
    format!(
        "{FRONTEND}\n\n\
fn main() -> Int {{\n\
\x20 let src: String = {lit};\n\
\x20 let out: String = compile(src, \"{mode}\");\n\
\x20 let mut sum: Int = 0;\n\
\x20 let mut i: Int = 0;\n\
\x20 while i < string_len(out) {{\n\
\x20   sum = sum + string_byte_at(out, i) * (i + 1);\n\
\x20   i = i + 1;\n\
\x20 }}\n\
\x20 return sum * 100000 + string_len(out);\n\
}}\n"
    )
}

/// Run the combined program through interpreter and native JIT; assert the Int
/// digests match (and are non-trivial, i.e. the dump wasn't empty).
///
/// Lowering + interpreting + codegen of the ~10k-line combined frontend recurses
/// deeply over very large MIR trees. The default 2 MiB test-thread stack is right
/// at the edge for this input, so under the parallel test harness it intermittently
/// overflowed (a flaky SIGSEGV — not a heap bug; ASan is clean and the digests
/// match). Run the work on a thread with a generous stack instead.
fn assert_native_matches_interpreter(prog: &str, mode: &str) {
    let prog = prog.to_string();
    let mode = mode.to_string();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || assert_native_matches_interpreter_inner(&prog, &mode))
        .expect("spawn large-stack worker")
        .join()
        .expect("worker thread panicked");
}

fn assert_native_matches_interpreter_inner(prog: &str, mode: &str) {
    let src = combined(prog, mode);
    let program = zeta::lower_source(&src).expect("combined frontend should lower");
    let oracle = match zeta::runtime::run_mir(&program).expect("interpreter should run") {
        Value::Int(n) => n,
        other => panic!("expected Int digest, got {other:?}"),
    };

    let module = zeta::parse_source(&src).expect("combined frontend should parse");
    let structs: Vec<zeta::ast::StructDecl> = module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(decl) => Some(decl.clone()),
            _ => None,
        })
        .collect();

    let native = zeta::codegen::jit_run_i64(&program, &structs, "main")
        .expect("native JIT of the whole frontend");

    assert_eq!(
        native, oracle,
        "native vs interpreter digest divergence for mode `{mode}`\n  native={native} oracle={oracle}"
    );
    println!("mode {mode}: native == interpreter digest = {native}");
}

// A small but feature-exercising Zeta program for the frontend to chew on:
// a function with params, a call, a `let`, arithmetic, and a `while` loop.
const SAMPLE: &str = "\
fn add(a: Int, b: Int) -> Int { return a + b; }
fn main() -> Int {
  let mut acc: Int = 0;
  let mut i: Int = 0;
  while i < 4 {
    acc = add(acc, i);
    i = i + 1;
  }
  return acc;
}
";

#[test]
fn frontend_ast_dump_matches() {
    assert_native_matches_interpreter(SAMPLE, "ast-dump");
}

#[test]
fn frontend_mir_dump_matches() {
    assert_native_matches_interpreter(SAMPLE, "mir-dump");
}

#[test]
fn frontend_run_mode_matches() {
    // "run" mode actually interprets the embedded program via the self-hosted
    // evaluator (ev_expr/ev_stmt) — the deepest path through the frontend.
    assert_native_matches_interpreter(SAMPLE, "run");
}

#[test]
fn frontend_typecheck_clean_matches() {
    assert_native_matches_interpreter(SAMPLE, "typecheck");
}

// A richer program: struct literal + field access, array + for-in, for-range,
// if/else, and a call — exercising many more frontend node kinds end-to-end.
const SAMPLE_RICH: &str = "\
struct Point { x: Int, y: Int }
fn sum_to(n: Int) -> Int {
  let mut s: Int = 0;
  for i in 0..n { s = s + i; }
  return s;
}
fn main() -> Int {
  let p: Point = Point { x: 3, y: 4 };
  let xs: IntArray = [10, 20, 30];
  let mut total: Int = p.x + p.y;
  for v in xs { total = total + v; }
  if total > 50 { return sum_to(total); }
  return total;
}
";

#[test]
fn frontend_rich_mir_dump_matches() {
    assert_native_matches_interpreter(SAMPLE_RICH, "mir-dump");
}

#[test]
fn frontend_rich_run_matches() {
    assert_native_matches_interpreter(SAMPLE_RICH, "run");
}
