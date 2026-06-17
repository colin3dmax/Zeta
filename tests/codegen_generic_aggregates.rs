// Native backend — generic struct/enum monomorphization (cargo feature `llvm`).
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_generic_aggregates
//
// Generic aggregates are monomorphized on demand from the value flow at each
// construction site: `Box<Int>` / `Option<Int>` become concrete LLVM layouts
// (`Box$Int`) / variant payload tables (`Option$Int`). Enums share the fixed
// `{tag, p0, p1}` layout; the inferred type argument drives payload encode/decode.
// For every program the JIT-compiled native result must equal the Stage0
// interpreter's (the differential oracle).
#![cfg(feature = "llvm")]

use zeta::ast::Item;
use zeta::runtime::Value;

fn check(source: &str) -> i64 {
    let module = zeta::parse_source(source).expect("should parse");
    let structs: Vec<zeta::ast::StructDecl> = module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(decl) => Some(decl.clone()),
            _ => None,
        })
        .collect();

    let program = zeta::lower_source(source).expect("should lower");
    let oracle = match zeta::runtime::run_mir(&program).expect("interpreter should run") {
        Value::Int(n) => n,
        other => panic!("expected Int, got {other:?}"),
    };
    let native = zeta::codegen::jit_run_i64(&program, &structs, "main").expect("native JIT");

    assert_eq!(
        native, oracle,
        "native/interpreter divergence\n--- source ---\n{source}\n--- native={native} oracle={oracle} ---"
    );
    oracle
}

#[test]
fn generic_struct_box() {
    let src = "\
struct Box<T> { value: T }
fn main() -> Int {
  let b: Box<Int> = Box { value: 42 };
  return b.value;
}";
    assert_eq!(check(src), 42);
}

#[test]
fn generic_enum_option_some() {
    let src = "\
enum Option<T> { Some(T), None }
fn main() -> Int {
  let o: Option<Int> = Option.Some(7);
  match o {
    Option.Some(x) -> { return x; },
    Option.None -> { return 0; },
  }
  return 0;
}";
    assert_eq!(check(src), 7);
}

#[test]
fn generic_enum_option_none() {
    let src = "\
enum Option<T> { Some(T), None }
fn main() -> Int {
  let o: Option<Int> = Option.None;
  match o {
    Option.Some(x) -> { return x; },
    Option.None -> { return 99; },
  }
  return 0;
}";
    assert_eq!(check(src), 99);
}

#[test]
fn generic_enum_result_ok() {
    let src = "\
enum Result<T, E> { Ok(T), Err(E) }
fn main() -> Int {
  let r: Result<Int, String> = Result.Ok(99);
  match r {
    Result.Ok(v) -> { return v; },
    Result.Err(e) -> { return 0; },
  }
  return 0;
}";
    assert_eq!(check(src), 99);
}

#[test]
fn generic_box_two_instances() {
    // Box<Int> and Box<String> coexist — two distinct monomorphized layouts
    // (i64 vs {len,ptr}) registered in one program. Only the Int box is read
    // back (a T-typed field can't be used in arithmetic/conditions, a language
    // -layer wildcard limitation), but both layouts must build without clashing.
    let src = "\
struct Box<T> { value: T }
fn main() -> Int {
  let a: Box<Int> = Box { value: 5 };
  let b: Box<String> = Box { value: \"hi\" };
  return a.value;
}";
    assert_eq!(check(src), 5);
}

#[test]
fn generic_string_payload() {
    // String payload in a generic enum exercises the {len, ptr} (p0, p1) split.
    let src = "\
import std.core;
enum Option<T> { Some(T), None }
fn main() -> Int {
  let o: Option<String> = Option.Some(\"hi\");
  match o {
    Option.Some(s) -> { return string_len(s); },
    Option.None -> { return 0; },
  }
  return 0;
}";
    assert_eq!(check(src), 2);
}
