// Operator overloading via traits: `a OP b` on a non-scalar operand dispatches
// to the operator's trait method (`+` → `add$Type`, `==` → `eq$Type`, ...),
// reusing UFCS/trait dispatch. Each program runs through BOTH the interpreter
// (oracle) and the native JIT; results must match.
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_operator
#![cfg(feature = "llvm")]

use zeta::ast::Item;
use zeta::runtime::Value;

fn check(source: &str) -> i64 {
    let structs: Vec<zeta::ast::StructDecl> = zeta::parse_source(source)
        .expect("parse")
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Struct(d) => Some(d.clone()),
            _ => None,
        })
        .collect();
    let program = zeta::lower_source(source).expect("lower");
    let oracle = match zeta::runtime::run_mir(&program).expect("interpreter") {
        Value::Int(n) => n,
        other => panic!("expected Int, got {other:?}"),
    };
    let native = zeta::codegen::jit_run_i64(&program, &structs, "main").expect("native JIT");
    assert_eq!(native, oracle, "native/interpreter divergence\n{source}");
    oracle
}

const VEC2: &str = "\
trait Add { fn add(self: Self, other: Self) -> Self; }
trait Sub { fn sub(self: Self, other: Self) -> Self; }
trait Eq { fn eq(self: Self, other: Self) -> Bool; }
struct Vec2 { x: Int, y: Int }
impl Add for Vec2 { fn add(self: Self, o: Self) -> Vec2 { return Vec2 { x: self.x + o.x, y: self.y + o.y }; } }
impl Sub for Vec2 { fn sub(self: Self, o: Self) -> Vec2 { return Vec2 { x: self.x - o.x, y: self.y - o.y }; } }
impl Eq for Vec2 { fn eq(self: Self, o: Self) -> Bool { return self.x == o.x; } }
";

#[test]
fn overloaded_add() {
    let src = format!(
        "{VEC2}
fn main() -> Int {{
  let a: Vec2 = Vec2 {{ x: 3, y: 4 }};
  let b: Vec2 = Vec2 {{ x: 10, y: 20 }};
  let c: Vec2 = a + b;          // {{13, 24}}
  return c.x + c.y;             // 37
}}"
    );
    assert_eq!(check(&src), 37);
}

#[test]
fn overloaded_sub_and_chain() {
    let src = format!(
        "{VEC2}
fn main() -> Int {{
  let a: Vec2 = Vec2 {{ x: 30, y: 40 }};
  let b: Vec2 = Vec2 {{ x: 10, y: 5 }};
  let c: Vec2 = Vec2 {{ x: 1, y: 2 }};
  let d: Vec2 = a - b + c;      // (20,35)+(1,2) = (21,37)
  return d.x + d.y;             // 58
}}"
    );
    assert_eq!(check(&src), 58);
}

#[test]
fn overloaded_eq_in_condition() {
    let src = format!(
        "{VEC2}
fn main() -> Int {{
  let a: Vec2 = Vec2 {{ x: 7, y: 1 }};
  let b: Vec2 = Vec2 {{ x: 7, y: 99 }};
  let mut r: Int = 0;
  if a == b {{ r = r + 100; }}   // eq compares x: 7==7 → true
  let c: Vec2 = Vec2 {{ x: 8, y: 1 }};
  if a == c {{ r = r + 1; }}     // 7==8 → false
  return r;                      // 100
}}"
    );
    assert_eq!(check(&src), 100);
}

#[test]
fn scalar_ops_unaffected() {
    // Built-in scalar operators must still use the fast path, not dispatch.
    let src = format!(
        "{VEC2}
fn main() -> Int {{
  let n: Int = 6 * 7 - 1;        // 41
  let f: Float = 2.5 + 1.5;      // 4.0
  let mut r: Int = n;
  if f > 3.0 {{ r = r + 1; }}
  return r;                      // 42
}}"
    );
    assert_eq!(check(&src), 42);
}
