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

use zeta::ast::Item;
use zeta::runtime::Value;

fn check(source: &str) -> i64 {
    // Struct-payload enums need the struct decls passed to the native backend; pull
    // them out of the parsed module (empty for the payload-less / scalar cases).
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
fn string_payload_construct_and_match() {
    // Result-like enum with a String payload; observe via string_len / byte_at.
    let src = "\
import std.core;
enum Msg { Text(String), Empty }
fn describe(m: Msg) -> Int {
  match m {
    Msg.Text(s) -> { return string_len(s) * 100 + string_byte_at(s, 0); }
    Msg.Empty -> { return -1; }
  }
  return 0;
}
fn main() -> Int {
  let a: Msg = Msg.Text(\"hello\");
  let b: Msg = Msg.Empty;
  return describe(a) * 10 + describe(b) + 1;
}";
    // describe(a): len 5 *100 + 'h'(104) = 604; describe(b) = -1
    // 604*10 + (-1) + 1 = 6040
    assert_eq!(check(src), 6040);
}

#[test]
fn string_payload_roundtrip_through_local() {
    let src = "\
import std.core;
enum Opt { Some(String), None }
fn main() -> Int {
  let o: Opt = Opt.Some(string_concat(\"ab\", \"cd\"));
  match o {
    Opt.Some(s) -> {
      if s == \"abcd\" { return 42; }
      return 7;
    }
    Opt.None -> { return 0; }
  }
  return 0;
}";
    assert_eq!(check(src), 42);
}

#[test]
fn mixed_payload_enum() {
    // Variants with Int, String, and no payload in one enum.
    let src = "\
import std.core;
enum Node { Num(Int), Name(String), Nil }
fn weight(n: Node) -> Int {
  match n {
    Node.Num(v) -> { return v * 2; }
    Node.Name(s) -> { return string_len(s); }
    Node.Nil -> { return 0; }
  }
  return 0;
}
fn main() -> Int {
  return weight(Node.Num(21)) * 1000 + weight(Node.Name(\"abcd\")) * 10 + weight(Node.Nil);
}";
    // 42*1000 + 4*10 + 0 = 42040
    assert_eq!(check(src), 42040);
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

// --- wide payloads: struct (heap-boxed) and array ({len,ptr} split) ---

#[test]
fn struct_payload_construct_and_match() {
    let src = "\
struct Point { x: Int, y: Int }
enum Shape { Dot(Point), Origin }
fn area(s: Shape) -> Int {
  match s {
    Shape.Dot(p) -> { return p.x * p.y; }
    Shape.Origin -> { return 0; }
  }
  return -1;
}
fn main() -> Int {
  let a: Shape = Shape.Dot(Point { x: 6, y: 7 });
  let b: Shape = Shape.Origin;
  return area(a) * 10 + area(b);
}";
    // 42*10 + 0
    assert_eq!(check(src), 420);
}

#[test]
fn struct_payload_wider_than_inline_slot() {
    // A 24-byte struct genuinely exceeds the inline (p0 + p1) = 16-byte slot, so
    // the heap box is exercised rather than coincidentally fitting.
    let src = "\
struct V3 { x: Int, y: Int, z: Int }
enum E { Vec(V3), Zero }
fn main() -> Int {
  let e: E = E.Vec(V3 { x: 1, y: 2, z: 3 });
  match e {
    E.Vec(v) -> { return v.x * 100 + v.y * 10 + v.z; }
    E.Zero -> { return 0; }
  }
  return -1;
}";
    assert_eq!(check(src), 123);
}

#[test]
fn struct_payload_is_value_independent() {
    // The boxed payload is a by-value copy: mutating the original struct local
    // after construction must not change the enum's snapshot.
    let src = "\
struct Counter { n: Int }
enum Snap { At(Counter), None }
fn main() -> Int {
  let mut c: Counter = Counter { n: 5 };
  let s: Snap = Snap.At(c);
  c.n = 99;
  match s {
    Snap.At(saved) -> { return saved.n * 10 + c.n; }
    Snap.None -> { return -1; }
  }
  return 0;
}";
    // snapshot 5*10 + mutated 99
    assert_eq!(check(src), 149);
}

#[test]
fn array_payload_sum() {
    let src = "\
import std.core;
enum Bag { Items(IntArray), Empty }
fn total(b: Bag) -> Int {
  match b {
    Bag.Items(xs) -> {
      let mut s: Int = 0;
      for x in xs { s = s + x; }
      return s * 10 + xs.len;
    }
    Bag.Empty -> { return -1; }
  }
  return 0;
}
fn main() -> Int {
  let mut xs: IntArray = int_array_empty();
  xs = int_array_push(xs, 3);
  xs = int_array_push(xs, 4);
  xs = int_array_push(xs, 5);
  return total(Bag.Items(xs)) * 100 + total(Bag.Empty) + 1;
}";
    // sum 12 *10 + len 3 = 123; 123*100 + (-1) + 1 = 12300
    assert_eq!(check(src), 12300);
}

#[test]
fn array_payload_is_value_independent() {
    // Constructing the enum deep-copies the buffer; a later in-place push on the
    // original array must not be visible through the enum.
    let src = "\
import std.core;
enum Box { Has(IntArray), Empty }
fn main() -> Int {
  let mut xs: IntArray = int_array_empty();
  xs = int_array_push(xs, 1);
  xs = int_array_push(xs, 2);
  let b: Box = Box.Has(xs);
  xs = int_array_push(xs, 3);
  let mut from_enum: Int = 0;
  match b {
    Box.Has(ys) -> { from_enum = ys.len; }
    Box.Empty -> { from_enum = -1; }
  }
  return from_enum * 10 + xs.len;
}";
    // enum kept [1,2] (len 2); xs grew to [1,2,3] (len 3)
    assert_eq!(check(src), 23);
}

#[test]
fn mixed_wide_and_scalar_payloads() {
    // One enum mixing Int, struct, array, and no-payload variants.
    let src = "\
import std.core;
struct Pair { a: Int, b: Int }
enum Cell { Num(Int), Pair2(Pair), List(IntArray), Nil }
fn val(c: Cell) -> Int {
  match c {
    Cell.Num(n) -> { return n; }
    Cell.Pair2(p) -> { return p.a + p.b; }
    Cell.List(xs) -> { return xs.len; }
    Cell.Nil -> { return 0; }
  }
  return -1;
}
fn main() -> Int {
  let mut xs: IntArray = int_array_empty();
  xs = int_array_push(xs, 9);
  xs = int_array_push(xs, 8);
  return val(Cell.Num(5)) * 1000
       + val(Cell.Pair2(Pair { a: 3, b: 4 })) * 100
       + val(Cell.List(xs)) * 10
       + val(Cell.Nil);
}";
    // 5*1000 + 7*100 + 2*10 + 0
    assert_eq!(check(src), 5720);
}
