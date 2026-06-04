#!/usr/bin/env node
const fs = require("node:fs");

const wasmPath = process.argv[2] || "target/wasm32-unknown-unknown/release/zeta.wasm";
const bytes = fs.readFileSync(wasmPath);
const wasmModule = new WebAssembly.Module(bytes);
const wasmExports = WebAssembly.Module.exports(wasmModule).map((entry) => entry.name).sort();
const required = ["memory", "zeta_alloc", "zeta_dealloc", "zeta_playground"];
const missing = required.filter((name) => !wasmExports.includes(name));

if (missing.length) {
  console.error(`missing wasm exports: ${missing.join(", ")}`);
  process.exit(1);
}

const instance = new WebAssembly.Instance(wasmModule, {});
const wasm = instance.exports;
const encoder = new TextEncoder();
const decoder = new TextDecoder();

function copy(bytes) {
  const ptr = wasm.zeta_alloc(bytes.length);
  if (!ptr) throw new Error("zeta_alloc returned null");
  new Uint8Array(wasm.memory.buffer, ptr, bytes.length).set(bytes);
  return ptr;
}

function unpack(value) {
  const packed = typeof value === "bigint" ? value : BigInt(value);
  return {
    ptr: Number(packed >> 32n),
    len: Number(packed & 0xffffffffn),
  };
}

function run(mode, source) {
  const modeBytes = encoder.encode(mode);
  const sourceBytes = encoder.encode(source);
  const modePtr = copy(modeBytes);
  const sourcePtr = copy(sourceBytes);
  try {
    const result = unpack(wasm.zeta_playground(modePtr, modeBytes.length, sourcePtr, sourceBytes.length));
    const text = decoder.decode(new Uint8Array(wasm.memory.buffer, result.ptr, result.len));
    wasm.zeta_dealloc(result.ptr, result.len);
    return JSON.parse(text);
  } finally {
    wasm.zeta_dealloc(modePtr, modeBytes.length);
    wasm.zeta_dealloc(sourcePtr, sourceBytes.length);
  }
}

const enumProgram = `enum ResultTag {
  Ok,
  Err,
}

fn main() -> Int {
  let tag: ResultTag = ResultTag.Ok;
  match tag {
    ResultTag.Ok -> { return 42; },
    ResultTag.Err -> { return 0; },
  }
  return 0;
}`;

const arrayProgram = `fn main() -> Int {
  let values: IntArray = [2, 4, 6];
  return values[0] + values[1] + values.len;
}`;

const stringScanProgram = `import std.core;

fn main() -> Int {
  let text: String = "A9 zeta";
  let first: Int = string_byte_at(text, 0);
  let digit: Int = string_byte_at(text, 1);
  let space: Int = string_byte_at(text, 2);
  let tail: String = string_byte_slice(text, 3, 4);
  if string_len(text) == 7 && ascii_is_alpha(first) && ascii_is_digit(digit) && ascii_is_whitespace(space) && string_len(tail) == 4 {
    return first + digit;
  }
  return 0;
}`;

const arrayBuilderProgram = `import std.core;

fn main() -> Int {
  let mut values: IntArray = int_array_empty();
  values = int_array_push(values, 2);
  values = int_array_push(values, 4);
  values = int_array_push(values, 6);
  return values[0] + values[1] + values.len;
}`;

const ioPathDiagnosticProgram = `import std.io;

fn main() -> String {
  let path: String = path_join("src", "main.zeta");
  return diagnostic_format("LEX_BAD_CHAR", 3, 5, path_basename(path));
}`;

const checks = [
  ["run", "fn main() -> Int { return 40 + 2; }", "42"],
  ["run", "struct User { name: String, age: Int, } fn main() -> Int { let user: User = User { name: \"Ada\", age: 42 }; return user.age; }", "42"],
  ["run", enumProgram, "42"],
  ["run", arrayProgram, "9"],
  ["run", stringScanProgram, "122"],
  ["run", arrayBuilderProgram, "9"],
  ["run", ioPathDiagnosticProgram, "LEX_BAD_CHAR at 3:5: main.zeta"],
  ["check", "fn main() -> Bool { return true && !false; }", "ok"],
  ["check-module-graph", "// file: main.zeta\nmodule demo.app;\nimport demo.math;\nfn main() -> Int { return answer(); }\n// file: math.zeta\nmodule demo.math;\nexport fn answer() -> Int { return 42; }\n", "ok"],
];

for (const [mode, source, expected] of checks) {
  const result = run(mode, source);
  if (!result.ok || result.output.trim() !== expected) {
    console.error(JSON.stringify({ mode, expected, result }, null, 2));
    process.exit(1);
  }
}

const size = fs.statSync(wasmPath).size;
console.log(JSON.stringify({ ok: true, wasm: wasmPath, size, exports: wasmExports }, null, 2));
