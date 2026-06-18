// Built-in generic `Option<T>` / `Result<T, E>` from `import std.core;`.
//
// Interpreter (run_mir) is always exercised; under the `llvm` feature the native
// JIT result must equal it. The built-ins are injected as external generic enums
// (std_api), flow through resolve/typecheck/mir, and native monomorphizes them.
use zeta::runtime::{run_mir, Value};

fn run(src: &str) -> i64 {
    let program = zeta::lower_source(src).expect("should lower");
    match run_mir(&program).expect("interpreter should run") {
        Value::Int(n) => n,
        other => panic!("expected Int, got {other:?}"),
    }
}

#[cfg(feature = "llvm")]
fn check(src: &str) -> i64 {
    let oracle = run(src);
    let program = zeta::lower_source(src).expect("should lower");
    let native = zeta::codegen::jit_run_i64(&program, &[], "main").expect("native JIT");
    assert_eq!(native, oracle, "native/interpreter divergence\n{src}");
    oracle
}

#[cfg(not(feature = "llvm"))]
fn check(src: &str) -> i64 {
    run(src)
}

#[test]
fn builtin_option_some() {
    let src = "\
import std.core;
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
fn builtin_option_none() {
    let src = "\
import std.core;
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
fn builtin_result_ok() {
    let src = "\
import std.core;
fn main() -> Int {
  let r: Result<Int, String> = Result.Ok(42);
  match r {
    Result.Ok(v) -> { return v; },
    Result.Err(e) -> { return 0; },
  }
  return 0;
}";
    assert_eq!(check(src), 42);
}

#[test]
fn builtin_result_across_function() {
    // The whole point: a function returning a built-in generic Result.
    let src = "\
import std.core;
fn parse(ok: Bool) -> Result<Int, String> {
  if ok { return Result.Ok(5); }
  return Result.Err(\"nope\");
}
fn main() -> Int {
  match parse(true) {
    Result.Ok(v) -> { return v; },
    Result.Err(e) -> { return 0; },
  }
  return 0;
}";
    assert_eq!(check(src), 5);
}

#[test]
fn legacy_optionint_still_works() {
    // The monomorphized legacy enums must keep working alongside the generics.
    let src = "\
import std.core;
fn main() -> Int {
  let o: OptionInt = OptionInt.Some(40);
  let r: ResultInt = ResultInt.Ok(2);
  match o {
    OptionInt.Some(a) -> {
      match r {
        ResultInt.Ok(b) -> { return a + b; },
        ResultInt.Err(m) -> { return 0; },
      }
      return 0;
    },
    OptionInt.None -> { return 0; },
  }
  return 0;
}";
    assert_eq!(check(src), 42);
}
