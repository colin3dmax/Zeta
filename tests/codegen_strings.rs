// std.strings — `string_split`, a pure-Zeta source-injected library function.
// Being pure Zeta (built from std.core string builtins), it lowers like user
// code and is correct on every backend by construction; these tests confirm the
// interpreter (oracle) and native JIT agree, and pin the split contract.
#![cfg(feature = "llvm")]

use zeta::runtime::Value;

/// Lower `source`, run interpreter + native JIT, assert agreement, return main's Int.
fn check(source: &str) -> i64 {
    let program = zeta::lower_source(source).expect("lower");
    let oracle = match zeta::runtime::run_mir(&program).expect("interpreter") {
        Value::Int(n) => n,
        other => panic!("expected Int, got {other:?}"),
    };
    let native = zeta::codegen::jit_run_i64(&program, &[], "main").expect("native JIT");
    assert_eq!(native, oracle, "native/interpreter divergence");
    oracle
}

/// A harness that splits `s` on `sep` and encodes the result as
/// `count * 1_000_000 + Σ (len(piece_i) + 1) * 7^i` — sensitive to both the
/// number of pieces and each piece's length (so an off-by-one or a dropped
/// empty piece changes the number).
fn split_encode(s: &str, sep: &str) -> String {
    format!(
        "\
import std.core;
import std.strings;
fn main() -> Int {{
  let parts: StringArray = string_split(\"{s}\", \"{sep}\");
  let mut acc: Int = parts.len * 1000000;
  let mut w: Int = 1;
  let mut i: Int = 0;
  while i < parts.len {{
    acc = acc + (string_len(parts[i]) + 1) * w;
    w = w * 7;
    i = i + 1;
  }}
  return acc;
}}"
    )
}

fn enc(pieces: &[&str]) -> i64 {
    let mut acc = pieces.len() as i64 * 1_000_000;
    let mut w = 1i64;
    for p in pieces {
        acc += (p.len() as i64 + 1) * w;
        w *= 7;
    }
    acc
}

#[test]
fn split_basic() {
    assert_eq!(check(&split_encode("a,b,c", ",")), enc(&["a", "b", "c"]));
}

#[test]
fn split_empty_pieces_between_adjacent_separators() {
    assert_eq!(check(&split_encode("a,,b", ",")), enc(&["a", "", "b"]));
}

#[test]
fn split_leading_and_trailing_separators() {
    assert_eq!(check(&split_encode(",a,", ",")), enc(&["", "a", ""]));
}

#[test]
fn split_no_separator_yields_whole_string() {
    assert_eq!(check(&split_encode("abc", "x")), enc(&["abc"]));
}

#[test]
fn split_empty_input_yields_one_empty_piece() {
    assert_eq!(check(&split_encode("", ",")), enc(&[""]));
}

#[test]
fn split_multi_char_separator() {
    assert_eq!(
        check(&split_encode("aXXbXXc", "XX")),
        enc(&["a", "b", "c"])
    );
}

#[test]
fn split_empty_separator_yields_whole_string() {
    // A 0-width separator can't make progress, so the whole string is one piece.
    assert_eq!(check(&split_encode("abc", "")), enc(&["abc"]));
}
