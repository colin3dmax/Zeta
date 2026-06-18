// The postfix `?` (try) operator: unwrap Option/Result, early-return on failure.
//
// Desugared (pre-resolve) into a `match` that moves the continuation into the
// success arm, so it reuses ordinary match/enum/return — interpreter and native
// must agree. Built-in `Option`/`Result` come from `import std.core;`.
//
// NOTE: a `?`-unwrapped value has the generic payload type `T` (a wildcard under
// the lenient generic typing — the interpreter is the oracle). It can be
// returned, stored, or passed to a function (assignment-position checks are
// lenient), but NOT used directly in arithmetic (`v + 1` is rejected — an
// operand constraint). These tests funnel unwrapped values through function
// calls, which is both within that rule and the realistic usage.
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

const RESULT_HELPERS: &str = "\
import std.core;
fn inc(x: Int) -> Int { return x + 1; }
fn half(n: Int) -> Result<Int, String> {
  if n % 2 == 0 { return Result.Ok(n / 2); }
  return Result.Err(\"odd\");
}
fn compute(n: Int) -> Result<Int, String> {
  let h = half(n)?;
  return Result.Ok(inc(h));
}
";

#[test]
fn result_try_success() {
    // half(10) = Ok(5) → ? unwraps to 5 → inc(5) = 6 → Ok(6).
    let src = format!(
        "{RESULT_HELPERS}
fn main() -> Int {{
  match compute(10) {{
    Result.Ok(v) -> {{ return v; }},
    Result.Err(e) -> {{ return 0 - 1; }},
  }}
  return 0;
}}"
    );
    assert_eq!(check(&src), 6);
}

#[test]
fn result_try_early_return() {
    // half(7) = Err("odd") → ? early-returns Err from compute → main's Err arm.
    let src = format!(
        "{RESULT_HELPERS}
fn main() -> Int {{
  match compute(7) {{
    Result.Ok(v) -> {{ return v; }},
    Result.Err(e) -> {{ return 42; }},
  }}
  return 0;
}}"
    );
    assert_eq!(check(&src), 42);
}

const OPTION_HELPERS: &str = "\
import std.core;
fn add_hundred(x: Int) -> Int { return x + 100; }
fn first_even(n: Int) -> Option<Int> {
  if n % 2 == 0 { return Option.Some(n); }
  return Option.None;
}
fn plus(n: Int) -> Option<Int> {
  let v = first_even(n)?;
  return Option.Some(add_hundred(v));
}
";

#[test]
fn option_try_some() {
    let src = format!(
        "{OPTION_HELPERS}
fn main() -> Int {{
  match plus(4) {{
    Option.Some(x) -> {{ return x; }},
    Option.None -> {{ return 0 - 1; }},
  }}
  return 0;
}}"
    );
    assert_eq!(check(&src), 104);
}

#[test]
fn option_try_none_early_return() {
    let src = format!(
        "{OPTION_HELPERS}
fn main() -> Int {{
  match plus(3) {{
    Option.Some(x) -> {{ return x; }},
    Option.None -> {{ return 7; }},
  }}
  return 0;
}}"
    );
    assert_eq!(check(&src), 7);
}

#[test]
fn try_in_expression_position() {
    // Two `?` in a single sub-expression — hoisted left-to-right; the unwrapped
    // values flow into a normal function call (lenient assignment position).
    let src = "\
import std.core;
fn add(a: Int, b: Int) -> Int { return a + b; }
fn g() -> Result<Int, String> { return Result.Ok(10); }
fn h() -> Result<Int, String> {
  let x = add(g()?, g()?);
  return Result.Ok(x);
}
fn main() -> Int {
  match h() {
    Result.Ok(v) -> { return v; },
    Result.Err(e) -> { return 0 - 1; },
  }
  return 0;
}";
    assert_eq!(check(src), 20);
}

#[test]
fn try_unwrapped_value_supports_arithmetic() {
    // The payoff of preserving generic args in typecheck: the `?`-unwrapped value
    // has the concrete success type (Int from Result<Int, String>), so it can be
    // used directly in arithmetic — `h + 1` no longer rejected.
    let src = "\
import std.core;
fn half(n: Int) -> Result<Int, String> {
  if n % 2 == 0 { return Result.Ok(n / 2); }
  return Result.Err(\"odd\");
}
fn compute(n: Int) -> Result<Int, String> {
  let h = half(n)?;
  return Result.Ok(h + 1);
}
fn main() -> Int {
  match compute(10) {
    Result.Ok(v) -> { return v; },
    Result.Err(e) -> { return 0 - 1; },
  }
  return 0;
}";
    assert_eq!(check(src), 6);
}

#[test]
fn match_generic_payload_is_concrete() {
    // Even without `?`: matching a concrete Result<Int,String> binds Ok's payload
    // as Int, usable in arithmetic.
    let src = "\
import std.core;
fn get() -> Result<Int, String> { return Result.Ok(20); }
fn main() -> Int {
  match get() {
    Result.Ok(v) -> { return v * 2; },
    Result.Err(e) -> { return 0; },
  }
  return 0;
}";
    assert_eq!(check(src), 40);
}

#[test]
fn try_outside_result_option_is_an_error() {
    // `?` in a function returning a plain `Int` has nothing to early-return into.
    let src = "\
import std.core;
fn g() -> Result<Int, String> { return Result.Ok(1); }
fn bad() -> Int {
  let x = g()?;
  return x;
}
fn main() -> Int { return bad(); }";
    let err = zeta::lower_source(src).expect_err("should be rejected");
    assert!(
        err.iter().any(|d| d.code == "DESUGAR_TRY_OUTSIDE_RESULT_OPTION"),
        "expected DESUGAR_TRY_OUTSIDE_RESULT_OPTION, got {err:?}"
    );
}

#[test]
fn try_chained_calls() {
    // `?` feeding directly into another `?`-returning call (unwrapped value
    // passed as an argument).
    let src = "\
import std.core;
fn step(n: Int) -> Result<Int, String> {
  if n > 100 { return Result.Err(\"too big\"); }
  return Result.Ok(n + 10);
}
fn pipeline(n: Int) -> Result<Int, String> {
  let a = step(n)?;
  let b = step(a)?;
  return Result.Ok(b);
}
fn main() -> Int {
  match pipeline(5) {
    Result.Ok(v) -> { return v; },
    Result.Err(e) -> { return 0 - 1; },
  }
  return 0;
}";
    assert_eq!(check(src), 25);
}
