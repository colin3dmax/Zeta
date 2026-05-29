import { ZETA_WASM_URL } from "./wasm-url.js";

let wasmPromise;

export async function runZeta(mode, source) {
  const wasm = await loadWasm();
  const modeBytes = new TextEncoder().encode(mode);
  const sourceBytes = new TextEncoder().encode(source);
  const modePtr = copyIntoWasm(wasm, modeBytes);
  const sourcePtr = copyIntoWasm(wasm, sourceBytes);

  try {
    const packed = wasm.zeta_playground(modePtr, modeBytes.length, sourcePtr, sourceBytes.length);
    const result = unpackResult(packed);
    const text = readString(wasm, result.ptr, result.len);
    wasm.zeta_dealloc(result.ptr, result.len);
    return JSON.parse(text);
  } finally {
    wasm.zeta_dealloc(modePtr, modeBytes.length);
    wasm.zeta_dealloc(sourcePtr, sourceBytes.length);
  }
}

async function loadWasm() {
  if (!wasmPromise) {
    wasmPromise = instantiateWasm().then(({ instance }) => instance.exports);
  }
  return wasmPromise;
}

async function instantiateWasm() {
  const response = await fetch(ZETA_WASM_URL);
  try {
    return await WebAssembly.instantiateStreaming(response.clone(), {});
  } catch (_) {
    const bytes = await response.arrayBuffer();
    return WebAssembly.instantiate(bytes, {});
  }
}

function copyIntoWasm(wasm, bytes) {
  const ptr = wasm.zeta_alloc(bytes.length);
  new Uint8Array(wasm.memory.buffer, ptr, bytes.length).set(bytes);
  return ptr;
}

function unpackResult(packed) {
  const value = typeof packed === "bigint" ? packed : BigInt(packed);
  return {
    ptr: Number(value >> 32n),
    len: Number(value & 0xffffffffn)
  };
}

function readString(wasm, ptr, len) {
  const bytes = new Uint8Array(wasm.memory.buffer, ptr, len);
  return new TextDecoder().decode(bytes);
}
