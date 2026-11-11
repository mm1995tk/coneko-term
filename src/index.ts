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
  type ClipboardItem,
  type ClipboardLocation,
  type ClipboardReadError,
  type ClipboardReadRequest,
  type ClipboardWriteRequest,
  type ClipboardWriteStatus,
  type DndErrorName,
  type DndEvent,
  type DndLocation,
  type DndOperation,
  type DndOperations,
  type DragImage,
  type DropDataForm,
  type DropRequest,
  type FileTransferErrorName,
  type FileTransferEvent,
  type FileTransferFileType,
  type FileTransferListing,
  type FileTransferStart,
  type CursorState,
  type ExtraCursorState,
  type MouseInput,
  type ProgramKind,
  type ProgramState,
  type ProgramStatus,
  type ProgramStatusChange,
  type NotificationSupport,
  type TerminalNotification,
  type WheelInput,
  type Disposable,
  type RowView,
  type SizedInfo,
  type TerminalOptions,
  type UrlClick,
  type VisibleUrl,
  type Theme,
} from "./terminal.js";

export {
  browserEnv,
  CanvasRenderer,
  Palette,
  SCROLLBAR_WIDTH,
  proposeDimensions,
  resolveScrollbarWidth,
  type CellSize,
  type FitInput,
  type RenderEnv,
  type RenderOptions,
  type RenderSource,
} from "./renderer.js";

export {
  PLACEMENT_STRIDE,
  Z_UNDER_BACKGROUND,
  ImageCache,
  browserImageDecoder,
  parsePlacements,
  type DrawableImage,
  type ImageDecoder,
  type ImagePlacement,
  type ImageSource,
} from "./images.js";

export {
  SelectionModel,
  type BufferPosition,
  type SelectionHost,
  type SelectionKind,
  type SelectionRange,
} from "./selection.js";

export {
  bracketTextForPaste,
  evaluateKeyboardEvent,
  KITTY_DISAMBIGUATE,
  KITTY_REPORT_ALL_KEYS,
  KITTY_REPORT_ALTERNATE_KEYS,
  KITTY_REPORT_EVENT_TYPES,
  KITTY_REPORT_TEXT,
  prepareTextForTerminal,
  type KeyEventLike,
  type KeyEventType,
  type KeyResult,
  type KeyResultType,
} from "./keyboard.js";

export {
  CompositionHelper,
  HostMoveWatcher,
  KeyHandler,
  type CompositionAnchor,
  type CompositionViewLike,
  type KeyHandlerHost,
  type TextareaLike,
} from "./input.js";
