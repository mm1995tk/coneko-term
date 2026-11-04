import wasmInit, {
  initSync,
  version as wasmVersion,
} from "./wasm/coneko_term_wasm.js";

export type WasmSource = BufferSource | WebAssembly.Module;

let ready = false;

/**
 * WebAssembly を読み込む。ほかの関数を呼ぶ前に 1 回呼ぶ。
 * 引数を省くと、パッケージ内の .wasm を fetch で読む。
 * Node.js などで fetch できないときは、.wasm のバイト列かモジュールを渡す。
 */
export async function init(source?: WasmSource): Promise<void> {
  if (ready) return;
  if (source === undefined) {
    await wasmInit();
  } else {
    initSync({ module: source });
  }
  ready = true;
}

/** coneko-term の版。`init()` のあとに呼ぶ。 */
export function version(): string {
  return wasmVersion();
}
