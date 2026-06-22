// Native backend — raw pointer type `*T` (cargo feature `llvm`).
//
//   LLVM_SYS_221_PREFIX=/opt/homebrew/opt/llvm \
//     cargo test --release --features llvm --test codegen_pointer
//
// Pointers are NATIVE-ONLY (the interpreter cannot deref a real address, so the
// usual interpreter-vs-native differential oracle does not apply). These tests
// JIT-compile and run, asserting the result directly. A valid, writable address
// is obtained from a heap array via `array_data_addr`, so the round-trips touch
// real memory the program owns.
#![cfg(feature = "llvm")]

use zeta::ast::Item;

fn run(source: &str) -> i64 {
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
    zeta::codegen::jit_run_i64(&program, &structs, "main").expect("native JIT")
}

#[test]
fn ptr_read_write_through_array_buffer() {
    // Write two Ints through a `*Int` into an array's buffer, read them back.
    let src = "\
import std.core;
fn main() -> Int {
  let xs: IntArray = array_repeat(0, 4);
  let base: Int = array_data_addr(xs);
  let p: *Int = ptr_from_addr(base);
  ptr_write(p, 111);
  let q: *Int = ptr_offset(p, 2);   // &xs[2]
  ptr_write(q, 222);
  let a: Int = ptr_read(p);          // 111
  let b: Int = ptr_read(q);          // 222
  // the writes must also be visible through normal array indexing
  return a + b + xs[0] + xs[2];      // 111 + 222 + 111 + 222 = 666
}";
    assert_eq!(run(src), 666);
}

#[test]
fn ptr_addr_roundtrip_and_offset_stride() {
    // ptr_addr ∘ ptr_from_addr is identity; ptr_offset strides by element size.
    let src = "\
import std.core;
fn main() -> Int {
  let xs: IntArray = array_repeat(7, 3);
  let base: Int = array_data_addr(xs);
  let p: *Int = ptr_from_addr(base);
  let q: *Int = ptr_offset(p, 1);
  // q's address is base + 8 (one i64 element).
  let stride: Int = ptr_addr(q) - base;      // 8
  let r: *Int = ptr_from_addr(ptr_addr(q));  // round-trip
  ptr_write(r, 99);
  return stride + ptr_read(q);               // 8 + 99 = 107
}";
    assert_eq!(run(src), 107);
}

#[test]
fn ptr_to_struct_reads_whole_value() {
    // A `*Point` reads/writes a whole struct value through the pointer.
    let src = "\
import std.core;
struct Point { x: Int, y: Int }
fn main() -> Int {
  // Back the struct with an array big enough to hold {x, y} (2 x i64).
  let buf: IntArray = array_repeat(0, 2);
  let p: *Point = ptr_from_addr(array_data_addr(buf));
  ptr_write(p, Point { x: 40, y: 2 });
  let pt: Point = ptr_read(p);
  return pt.x + pt.y;   // 42
}";
    assert_eq!(run(src), 42);
}
