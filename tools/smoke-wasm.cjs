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

const checks = [
  ["run", "fn main() -> Int { return 40 + 2; }", "42"],
  ["run", "struct User { name: String, age: Int, } fn main() -> Int { let user: User = User { name: \"Ada\", age: 42 }; return user.age; }", "42"],
  ["run", enumProgram, "42"],
  ["check", "fn main() -> Bool { return true && !false; }", "ok"],
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
