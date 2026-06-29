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

/// Deterministic checksum of a string: `len * 1_000_000 + Σ byte_i * (i + 1)`.
/// Used to compare a String result across backends as an Int.
fn checksum(s: &str) -> i64 {
    let mut acc = s.len() as i64 * 1_000_000;
    for (i, b) in s.bytes().enumerate() {
        acc += b as i64 * (i as i64 + 1);
    }
    acc
}

/// A harness whose `main` builds a String via `expr` and returns its checksum
/// (matching `checksum`), so the differential `check` compares the actual bytes.
fn string_result(expr: &str) -> String {
    format!(
        "\
import std.core;
import std.strings;
fn main() -> Int {{
  let s: String = {expr};
  let mut acc: Int = string_len(s) * 1000000;
  let mut i: Int = 0;
  while i < string_len(s) {{
    acc = acc + string_byte_at(s, i) * (i + 1);
    i = i + 1;
  }}
  return acc;
}}"
    )
}

/// A harness returning an Int directly (for the Bool predicates, encoded 0/1).
fn int_result(body_expr: &str) -> String {
    format!(
        "\
import std.core;
import std.strings;
fn b2i(b: Bool) -> Int {{ if b {{ return 1; }} return 0; }}
fn main() -> Int {{ return {body_expr}; }}"
    )
}

#[test]
fn join_is_inverse_of_split() {
    assert_eq!(
        check(&string_result("string_join(string_split(\"a,b,c\", \",\"), \"-\")")),
        checksum("a-b-c")
    );
}

#[test]
fn join_empty_and_single() {
    // Empty array -> "", single piece -> itself (no separator added).
    assert_eq!(
        check(&string_result("string_join(string_array_empty(), \",\")")),
        checksum("")
    );
    assert_eq!(
        check(&string_result("string_join(string_split(\"solo\", \",\"), \"+\")")),
        checksum("solo")
    );
}

#[test]
fn replace_all_occurrences() {
    assert_eq!(
        check(&string_result("string_replace(\"a.b.b.c\", \"b\", \"X\")")),
        checksum("a.X.X.c")
    );
}

#[test]
fn replace_multi_char_and_grow() {
    // Replacement longer than the match, multiple hits.
    assert_eq!(
        check(&string_result("string_replace(\"xAxAx\", \"A\", \"--\")")),
        checksum("x--x--x")
    );
}

#[test]
fn replace_empty_from_is_identity() {
    assert_eq!(
        check(&string_result("string_replace(\"abc\", \"\", \"X\")")),
        checksum("abc")
    );
}

#[test]
fn starts_with_and_ends_with() {
    // 1*1000 + 0*100 + 1*10 + 0 = 1010 (he✓, lo✗ as prefix; lo✓, hello✗ as suffix).
    let body = "b2i(string_starts_with(\"hello\", \"he\")) * 1000 \
                + b2i(string_starts_with(\"hello\", \"lo\")) * 100 \
                + b2i(string_ends_with(\"hello\", \"lo\")) * 10 \
                + b2i(string_ends_with(\"hi\", \"hello\"))";
    assert_eq!(check(&int_result(body)), 1010);
}

#[test]
fn empty_prefix_suffix_always_match() {
    let body = "b2i(string_starts_with(\"x\", \"\")) * 10 + b2i(string_ends_with(\"x\", \"\"))";
    assert_eq!(check(&int_result(body)), 11);
}

#[test]
fn trim_start_drops_leading_whitespace() {
    assert_eq!(
        check(&string_result("string_trim_start(\"   hi  \")")),
        checksum("hi  ")
    );
}

#[test]
fn trim_end_drops_trailing_whitespace() {
    assert_eq!(
        check(&string_result("string_trim_end(\"   hi  \")")),
        checksum("   hi")
    );
}

#[test]
fn trim_handles_tab_newline_formfeed_set() {
    // \t \n \r \x0c form-feed are all whitespace (matching the string_trim builtin).
    assert_eq!(
        check(&string_result("string_trim_start(\"\\t\\n\\r x\")")),
        checksum("x")
    );
    assert_eq!(
        check(&string_result("string_trim_end(\"x \\t\\n\\r\")")),
        checksum("x")
    );
}

#[test]
fn trim_all_whitespace_yields_empty() {
    // Forces the scan index to the end — confirms `&&` short-circuits (no
    // out-of-bounds byte read) on BOTH backends.
    assert_eq!(check(&string_result("string_trim_start(\"     \")")), checksum(""));
    assert_eq!(check(&string_result("string_trim_end(\"     \")")), checksum(""));
}

#[test]
fn trim_no_whitespace_is_unchanged() {
    assert_eq!(
        check(&string_result("string_trim_start(\"abc\")")),
        checksum("abc")
    );
    assert_eq!(
        check(&string_result("string_trim_end(\"abc\")")),
        checksum("abc")
    );
}
