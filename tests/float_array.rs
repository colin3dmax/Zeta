// FloatArray (P3 roadmap, first slice): Float as an array element type.
// Differential against the interpreter oracle (run_mir).
use zeta::runtime::{run_mir, Value};

fn run(src: &str) -> Value {
    let p = zeta::lower_source(src).expect("lower");
    run_mir(&p).expect("run")
}

#[test]
fn float_array_literal_and_index() {
    let src = "\
fn main() -> Int {
  let xs: FloatArray = [1.5, 2.5, 3.0];
  let s: Float = xs[0] + xs[1] + xs[2];
  if s > 6.9 { if s < 7.1 { return xs.len; } }
  return 0;
}";
    assert_eq!(run(src), Value::Int(3));
}

#[test]
fn float_array_builder() {
    let src = "\
import std.core;
fn main() -> Int {
  let mut xs: FloatArray = float_array_empty();
  xs = float_array_push(xs, 1.5);
  xs = float_array_push(xs, 2.5);
  let s: Float = xs[0] + xs[1];
  if s > 3.9 { if s < 4.1 { return xs.len; } }
  return 0;
}";
    assert_eq!(run(src), Value::Int(2));
}
