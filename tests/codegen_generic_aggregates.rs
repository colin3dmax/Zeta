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
fn generic_array_field_and_param() {
    // `Array<T>` field in a generic struct + a generic function over `Array<T>`,
    // monomorphized to a concrete element layout per instantiation.
    let src = "\
struct Bag<T> { items: Array<T>, count: Int }
fn first<T>(xs: Array<T>) -> T { return xs[0]; }
fn main() -> Int {
  let b: Bag<Int> = Bag { items: [10, 20, 30], count: 3 };
  return first(b.items) + b.items[2] + b.count;
}";
    assert_eq!(check(src), 10 + 30 + 3);
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
fn generic_enum_return_across_function() {
    // The Stage B payoff: a function whose RETURN type is a generic enum
    // instantiation. The annotation `Option<Int>` is preserved through the
    // pipeline so native resolves the function signature to the `Option$Int`
    // instance (not a degraded Int), and the caller can match on the result.
    let src = "\
enum Option<T> { Some(T), None }
fn find(hit: Bool) -> Option<Int> {
  if hit { return Option.Some(42); }
  return Option.None;
}
fn main() -> Int {
  match find(true) {
    Option.Some(x) -> { return x; },
    Option.None -> { return 0; },
  }
  return 0;
}";
    assert_eq!(check(src), 42);
}

#[test]
fn generic_result_return_string_err() {
    // Result<Int, String> across a function boundary: the Err payload is a
    // String, so the annotation-derived `Result$Int_Str` instance must carry the
    // correct payload type for the Err arm's {len,ptr} extraction.
    // Uses the built-in Result from std.core (defining one locally would conflict
    // with the reserved std.core name).
    let src = "\
import std.core;
fn parse(ok: Bool) -> Result<Int, String> {
  if ok { return Result.Ok(7); }
  return Result.Err(\"bad\");
}
fn main() -> Int {
  match parse(false) {
    Result.Ok(v) -> { return v; },
    Result.Err(e) -> { return string_len(e); },
  }
  return 0;
}";
    assert_eq!(check(src), 3);
}

#[test]
fn generic_struct_return_across_function() {
    // A function returning a generic struct instance `Box<Int>`.
    let src = "\
struct Box<T> { value: T }
fn wrap(n: Int) -> Box<Int> { return Box { value: n }; }
fn main() -> Int {
  let b: Box<Int> = wrap(11);
  return b.value;
}";
    assert_eq!(check(src), 11);
}

#[test]
fn generic_aggregate_param() {
    // A generic aggregate passed as a parameter `Box<Int>`.
    let src = "\
struct Box<T> { value: T }
fn unwrap(b: Box<Int>) -> Int { return b.value; }
fn main() -> Int {
  let b: Box<Int> = Box { value: 8 };
  return unwrap(b);
}";
    assert_eq!(check(src), 8);
}

#[test]
fn generic_string_payload() {
    // String payload in a generic enum exercises the {len, ptr} (p0, p1) split.
    // Uses the built-in Option from std.core.
    let src = "\
import std.core;
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
