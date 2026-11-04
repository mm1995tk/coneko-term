let memory: WebAssembly.Memory | undefined;

/** `init()` が WASM のメモリを渡す。 */
export function setMemory(m: WebAssembly.Memory): void {
  memory = m;
}

/** WASM のメモリ。`init()` の前は例外。 */
export function getMemory(): WebAssembly.Memory {
  if (memory === undefined) {
    throw new Error("coneko-term: call init() before using Terminal");
  }
  return memory;
}
