import wasmInit, { version as wasmVersion } from "./wasm/coneko_term_wasm.js";

/**
 * `init()` に渡せる .wasm。
 * .wasm のバイト列（`BufferSource`）か、コンパイル済みの `WebAssembly.Module`。
 */
export type WasmSource = BufferSource | WebAssembly.Module;

/**
 * WebAssembly を読み込む。ほかの関数を呼ぶ前に呼ぶ。2 回目以降は何もしない。
 * 引数を省くと、パッケージ内の .wasm を fetch で読む。
 * Node.js などで fetch できないときは、.wasm のバイト列かモジュールを渡す。
 */
export async function init(source?: WasmSource): Promise<void> {
  await wasmInit(source === undefined ? undefined : { module_or_path: source });
}

/** coneko-term の版。`init()` のあとに呼ぶ。 */
export function version(): string {
  return wasmVersion();
}
