// Generic struct/enum (P3, language layer): declarations + construction + use,
// interpreter-only (instantiation args erased, type params typed leniently).
// run_mir is the oracle.
use zeta::runtime::{run_mir, Value};

fn run(src: &str) -> Value {
    let p = zeta::lower_source(src).expect("lower");
    run_mir(&p).expect("run")
}

#[test]
fn generic_struct_box() {
    let src = "\
struct Box<T> { value: T }
fn main() -> Int {
  let b: Box<Int> = Box { value: 42 };
  return b.value;
}";
    assert_eq!(run(src), Value::Int(42));
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
    assert_eq!(run(src), Value::Int(7));
}

#[test]
fn generic_enum_result() {
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
    assert_eq!(run(src), Value::Int(99));
}
