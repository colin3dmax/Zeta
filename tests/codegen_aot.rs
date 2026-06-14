// Native backend — AOT: compile a Zeta program to a native OBJECT file, link it
// into a standalone executable with `cc`, run it, and check the output equals
// the interpreter (the differential oracle). This is the JIT-free path: Zeta →
// real binary, a step toward dropping Stage0.
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_aot
#![cfg(feature = "llvm")]

use std::io::Write;
use std::process::Command;

use zeta::ast::Item;
use zeta::runtime::Value;

// Calls the AOT-compiled entry (renamed `zeta_entry`) and prints its Int result.
const DRIVER: &str = "\
#include <stdio.h>
#include <stdint.h>
extern int64_t zeta_entry(void);
int main(void) { printf(\"%lld\\n\", (long long) zeta_entry()); return 0; }
";

/// Compile `source` to an object, link a standalone exe, run it, and assert the
/// printed result equals the interpreter's. `tag` keeps temp paths unique so
/// parallel tests don't clobber each other.
fn check_aot(tag: &str, source: &str) -> i64 {
    let module = zeta::parse_source(source).expect("parse");
    let structs: Vec<zeta::ast::StructDecl> = module
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Struct(decl) => Some(decl.clone()),
            _ => None,
        })
        .collect();
    let program = zeta::lower_source(source).expect("lower");
    let oracle = match zeta::runtime::run_mir(&program).expect("interpret") {
        Value::Int(n) => n,
        other => panic!("expected Int, got {other:?}"),
    };

    let dir = std::env::temp_dir();
    let obj = dir.join(format!("zeta_aot_{tag}.o"));
    let drv = dir.join(format!("zeta_aot_{tag}.c"));
    let exe = dir.join(format!("zeta_aot_{tag}"));

    zeta::codegen::aot_compile_object(&program, &structs, "main", &obj).expect("aot object");
    std::fs::File::create(&drv)
        .unwrap()
        .write_all(DRIVER.as_bytes())
        .unwrap();

    let linked = Command::new("cc")
        .arg(&obj)
        .arg(&drv)
        .arg("-o")
        .arg(&exe)
        .status()
        .expect("cc link");
    assert!(linked.success(), "linking the AOT object should succeed");

    let out = Command::new(&exe).output().expect("run exe");
    assert!(out.status.success(), "AOT executable should run");
    let printed: i64 = String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .expect("exe should print an integer");

    assert_eq!(
        printed, oracle,
        "AOT binary / interpreter divergence\n--- source ---\n{source}\n--- aot={printed} oracle={oracle} ---"
    );
    oracle
}

#[test]
fn aot_scalar() {
    let src = "\
fn fact(n: Int) -> Int { if n <= 1 { return 1; } return n * fact(n - 1); }
fn main() -> Int {
  let mut i: Int = 0;
  let mut sum: Int = 0;
  while i < 10 { sum = sum + i * i; i = i + 1; }
  return sum + fact(5);
}";
    // sum of i*i for 0..9 = 285; fact(5)=120 → 405
    assert_eq!(check_aot("scalar", src), 405);
}

#[test]
fn aot_struct() {
    let src = "\
struct Point { x: Int, y: Int }
fn make(a: Int, b: Int) -> Point { return Point { x: a, y: b }; }
fn main() -> Int {
  let mut p: Point = make(3, 4);
  p.x = p.x + 10;
  return p.x * p.y;
}";
    assert_eq!(check_aot("struct", src), 52);
}

#[test]
fn aot_array() {
    // Exercises malloc/memcpy (linked from libc) in a standalone binary.
    let src = "\
fn main() -> Int {
  let mut xs: IntArray = [0, 0, 0, 0, 0];
  let mut i: Int = 0;
  while i < xs.len { xs[i] = i * 2; i = i + 1; }
  let a: IntArray = xs;
  return a[0] + a[1] + a[2] + a[3] + a[4] + a.len;
}";
    // 0+2+4+6+8 = 20, + len 5 = 25
    assert_eq!(check_aot("array", src), 25);
}
