// Native backend — enum + match subset differential tests (cargo feature `llvm`).
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_enum
//
// Enums are `{ i64 tag, i64 payload }` (Int / no-payload variants); `match` lowers
// to an LLVM `switch` over the tag (or over an Int/Bool value). The Stage0
// interpreter is the differential oracle. Enum decls come from `program.enums`, so
// no struct decls are needed.
#![cfg(feature = "llvm")]

use zeta::runtime::Value;

fn check(source: &str) -> i64 {
    let program = zeta::lower_source(source).expect("should lower");
    let oracle = match zeta::runtime::run_mir(&program).expect("interpreter should run") {
        Value::Int(n) => n,
        other => panic!("expected Int, got {other:?}"),
    };
    let native = zeta::codegen::jit_run_i64(&program, &[], "main").expect("native JIT");
    assert_eq!(
        native, oracle,
        "native/interpreter divergence\n--- source ---\n{source}\n--- native={native} oracle={oracle} ---"
    );
    oracle
}

#[test]
fn payloadless_match_picks_variant() {
    let src = "\
enum ResultTag { Ok, Err }
fn main() -> Int {
  let tag: ResultTag = ResultTag.Ok;
  match tag {
    ResultTag.Ok -> { return 42; }
    ResultTag.Err -> { return 0; }
  }
  return -1;
}";
    assert_eq!(check(src), 42);
}

#[test]
fn payloadless_match_second_variant() {
    let src = "\
enum ResultTag { Ok, Err }
fn main() -> Int {
  let tag: ResultTag = ResultTag.Err;
  match tag {
    ResultTag.Ok -> { return 42; }
    ResultTag.Err -> { return 7; }
  }
  return -1;
}";
    assert_eq!(check(src), 7);
}

#[test]
fn three_variant_tag_dispatch() {
    let src = "\
enum Color { Red, Green, Blue }
fn code(c: Color) -> Int {
  match c {
    Color.Red -> { return 1; }
    Color.Green -> { return 2; }
    Color.Blue -> { return 3; }
  }
  return 0;
}
fn main() -> Int {
  return code(Color.Red) * 100 + code(Color.Green) * 10 + code(Color.Blue);
}";
    assert_eq!(check(src), 123);
}

#[test]
fn int_payload_bind_and_use() {
    let src = "\
enum OptionInt { Some(Int), None }
fn unwrap_or(value: OptionInt, fallback: Int) -> Int {
  match value {
    OptionInt.Some(answer) -> { return answer; }
    OptionInt.None -> { return fallback; }
  }
  return 0;
}
fn main() -> Int {
  let a: OptionInt = OptionInt.Some(40);
  let b: OptionInt = OptionInt.None;
  return unwrap_or(a, 99) + unwrap_or(b, 99);
}";
    // 40 + 99
    assert_eq!(check(src), 139);
}

#[test]
fn payload_arithmetic() {
    let src = "\
enum OptionInt { Some(Int), None }
fn main() -> Int {
  let x: OptionInt = OptionInt.Some(10);
  match x {
    OptionInt.Some(n) -> { return n * n + 5; }
    OptionInt.None -> { return 0; }
  }
  return 0;
}";
    assert_eq!(check(src), 105);
}

#[test]
fn match_with_wildcard_default() {
    let src = "\
enum Dir { North, East, South, West }
fn main() -> Int {
  let d: Dir = Dir.South;
  match d {
    Dir.North -> { return 1; }
    _ -> { return 99; }
  }
  return 0;
}";
    assert_eq!(check(src), 99);
}

#[test]
fn match_over_int_value() {
    let src = "\
fn classify(n: Int) -> Int {
  match n {
    0 -> { return 100; }
    1 -> { return 200; }
    _ -> { return 300; }
  }
  return 0;
}
fn main() -> Int {
  return classify(0) + classify(1) + classify(5);
}";
    // 100 + 200 + 300
    assert_eq!(check(src), 600);
}

#[test]
fn match_over_bool_value() {
    let src = "\
fn pick(b: Bool) -> Int {
  match b {
    true -> { return 11; }
    false -> { return 22; }
  }
  return 0;
}
fn main() -> Int {
  return pick(true) * 100 + pick(false);
}";
    assert_eq!(check(src), 1122);
}

#[test]
fn match_name_binding_default() {
    // A `Name` pattern binds the whole scrutinee and acts as the default arm.
    let src = "\
fn f(n: Int) -> Int {
  match n {
    7 -> { return 0; }
    other -> { return other * 2; }
  }
  return 0;
}
fn main() -> Int {
  return f(7) + f(21);
}";
    // f(7)=0, f(21)=42
    assert_eq!(check(src), 42);
}

#[test]
fn match_falls_through_when_arms_dont_return() {
    // Arms assign a local instead of returning; code after the match uses it.
    // Exercises the `match.end` fall-through path.
    let src = "\
enum Sign { Neg, Zero, Pos }
fn main() -> Int {
  let s: Sign = Sign.Pos;
  let mut score: Int = 0;
  match s {
    Sign.Neg -> { score = 1; }
    Sign.Zero -> { score = 2; }
    Sign.Pos -> { score = 3; }
  }
  return score * 10;
}";
    assert_eq!(check(src), 30);
}

#[test]
fn match_in_loop() {
    let src = "\
enum Step { Inc, Dec }
fn main() -> Int {
  let mut acc: Int = 0;
  let mut i: Int = 0;
  while i < 5 {
    let s: Step = Step.Inc;
    match s {
      Step.Inc -> { acc = acc + 2; }
      Step.Dec -> { acc = acc - 1; }
    }
    i = i + 1;
  }
  return acc;
}";
    assert_eq!(check(src), 10);
}
