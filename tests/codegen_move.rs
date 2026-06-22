// Native backend — move-on-last-use correctness (cargo feature `llvm`).
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_move
//
// Each program runs through BOTH the interpreter (which never moves — pure
// value semantics) and the native JIT (which moves dead-after managed reads and
// suppresses the moved local's drop via a runtime flag). The results must match,
// which catches a wrong move that double-frees, leaks, or reads after free.
// String values are used throughout so the managed clone/drop paths are live.
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

const HELPERS: &str = "\
import std.core;
fn consume(s: String) -> Int { return string_len(s); }
";

#[test]
fn move_into_call_arg_last_use() {
    // x's only use is the call arg → moved (no clone); consume drops it once.
    let src = format!(
        "{HELPERS}
fn main() -> Int {{
  let x: String = string_concat(\"foo\", \"bar\");
  return consume(x);
}}"
    );
    assert_eq!(check(&src), 6);
}

#[test]
fn no_move_when_used_twice() {
    // First read is NOT dead-after (x is read again) → must clone; second moves.
    let src = format!(
        "{HELPERS}
fn main() -> Int {{
  let x: String = string_concat(\"hi\", \"!\");
  let a: Int = consume(x);
  let b: Int = consume(x);
  return a + b;
}}"
    );
    assert_eq!(check(&src), 6); // 3 + 3
}

#[test]
fn conditional_move_then_reassign() {
    // x moved on the then-path; the later reassignment's old-value drop must be
    // flag-guarded (skip on the moved path, drop on the else path), then the
    // flag is reset so the second consume moves the fresh value.
    let src = format!(
        "{HELPERS}
fn run(c: Int) -> Int {{
  let mut x: String = string_concat(\"aa\", \"bb\");
  let mut t: Int = 0;
  if c > 0 {{
    t = t + consume(x);
  }}
  x = string_concat(\"cc\", \"dd\");
  t = t + consume(x);
  return t;
}}
fn main() -> Int {{
  return run(1) * 100 + run(0);
}}"
    );
    // c>0: 4 (then) + 4 (after) = 8; c==0: only 4 → 8*100 + 4 = 804.
    assert_eq!(check(&src), 804);
}

#[test]
fn loop_local_move_each_iteration() {
    // A body-local string is built and moved each iteration; the per-iteration
    // flag reset keeps the drop correct across the back-edge.
    let src = format!(
        "{HELPERS}
fn main() -> Int {{
  let mut total: Int = 0;
  let mut i: Int = 0;
  while i < 50 {{
    let s: String = string_concat(\"ab\", \"c\");
    total = total + consume(s);
    i = i + 1;
  }}
  return total;
}}"
    );
    assert_eq!(check(&src), 150); // 50 * 3
}

#[test]
fn move_into_struct_field() {
    // The struct literal takes ownership of `name` (last use) without cloning.
    let src = "\
import std.core;
struct Person { name: String, age: Int }
fn make(n: String, a: Int) -> Person { return Person { name: n, age: a }; }
fn main() -> Int {
  let who: String = string_concat(\"Ada\", \"!\");
  let p: Person = make(who, 36);
  return string_len(p.name) + p.age;
}";
    assert_eq!(check(src), 4 + 36); // len("Ada!")=4
}

#[test]
fn move_param_rebind_chain() {
    // `let map = m` moves the param; threading through several rebinds must keep
    // exactly one live owner (no double-free, no leak-driven divergence).
    let src = "\
import std.core;
fn relabel(m: String) -> String {
  let a: String = m;
  let b: String = a;
  return b;
}
fn main() -> Int {
  let s: String = string_concat(\"xy\", \"z\");
  return string_len(relabel(s));
}";
    assert_eq!(check(src), 3);
}
