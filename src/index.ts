import wasmInit, { version as wasmVersion } from "./wasm/coneko_term_wasm.js";
import { setMemory } from "./memory.js";

/**
 * `init()` に渡せる .wasm。
 * .wasm のバイト列（`BufferSource`）か、コンパイル済みの `WebAssembly.Module`。
 */
export type WasmSource = BufferSource | WebAssembly.Module;

/**
 * WebAssembly を読み込む。ほかの関数や `Terminal` を使う前に呼ぶ。2 回目以降は何もしない。
 * 引数を省くと、パッケージ内の .wasm を fetch で読む。
 * Node.js などで fetch できないときは、.wasm のバイト列かモジュールを渡す。
 */
export async function init(source?: WasmSource): Promise<void> {
  const out = await wasmInit(
    source === undefined ? undefined : { module_or_path: source },
  );
  setMemory(out.memory);
}

/** coneko-term の版。`init()` のあとに呼ぶ。 */
export function version(): string {
  return wasmVersion();
}

export {
  CELL_STRIDE,
  CLUSTER_FLAG,
  Terminal,
  type CursorState,
  type Disposable,
  type RowView,
  type TerminalOptions,
  type Theme,
} from "./terminal.js";

export {
  browserEnv,
  CanvasRenderer,
  Palette,
  SCROLLBAR_WIDTH,
  proposeDimensions,
  type CellSize,
  type FitInput,
  type RenderEnv,
  type RenderOptions,
  type RenderSource,
} from "./renderer.js";
