# coneko-term

**coneko-term is a separate project from kitty.** It aims to implement all 14 items of kitty's [protocol extensions](https://sw.kovidgoyal.net/kitty/protocol-extensions/), but it is not made by or affiliated with kitty.

coneko-term is a library of terminal building blocks. The first user is the GUI of [shijimi-editor](https://github.com/mm1995tk/shijimi-editor), where it replaces xterm.js 6.0.0.

- The VT processing and the screen (parser, cells, cursor, scrollback) are written in Rust and compiled to WebAssembly.
- Drawing is TypeScript on Canvas 2D. Glyphs are drawn with the browser's fonts.
- The Rust core does not depend on the browser, so a native GPU renderer (wgpu and so on) can use the same core later.

The first goal is that shijimi-editor's GUI works with coneko-term as it does with xterm.js. kitty's protocol comes after that. "The same as xterm.js" is measured against the [xterm.js 6.0.0 feature table](https://xtermjs.org/docs/api/vtfeatures/) and its source.

**Status:** the WASM and TypeScript API (`Terminal`) and Canvas 2D drawing (with `fit()` and a scrollbar) are in place. Mouse reporting, the wheel, and selection with copy are in place. The kitty keyboard protocol is partly in place: the mode flags (`CSI > u`, `CSI < u`, `CSI = u`, `CSI ? u`) and all five encoding flags (1 disambiguate escape codes, 2 report event types, 4 report alternate keys, 8 report all keys as escape codes, 16 report associated text). Not done: the lock modifiers (Caps Lock and Num Lock bits), Hyper and Meta, and media keys. The core receives images sent with the kitty graphics protocol (APC `a=t`, `a=T` and `a=q`, chunked, direct transmission only), keeps them, places them on the cells (`a=p`, `a=T`) and deletes them (`a=d`), and lists the placements for the renderer. The renderer draws them on the Canvas: `f=24`, `f=32` and `f=100`, with or without `o=z` (PNG and zlib are decoded in the browser with `createImageBitmap` and `DecompressionStream`; an image that cannot be decoded is not drawn). With `z` below 0 they go under the text, and with `z` below -1073741824 under the cell background colors. Unicode placeholders (`U=1` virtual placements and U+10EEEE, with the row and column diacritics) are drawn too, including the rules for omitted diacritics. Animation is not done yet.

## Packages

- crate `coneko-term` (`coneko_term` in code): the core, in `crates/coneko-term`.
- npm package `coneko-term`: the WASM and the TypeScript types. The built `.wasm` is in the package, so it works with `npm --ignore-scripts`.

```ts
import { init, Terminal } from "coneko-term";

await init();
const term = new Terminal({ cols: 80, rows: 24, scrollback: 1000 });
term.onData((bytes) => pty.write(bytes)); // replies (DSR and so on) and input() go out here
term.registerOscHandler(133, (data) => console.log("OSC 133", data));
term.write("hello\r\n", () => console.log("processed"));
```

`Terminal` has no DOM dependency until you call `open(host)`. Large `write`s are processed in small slices, and `done` is called when that data has been processed. `takeDirtyRows()` and `readRow(y)` give the changed rows as cells in WASM memory.

## Drawing

```ts
const term = new Terminal({ fontFamily: "'JetBrains Mono', monospace", fontSize: 15, lineHeight: 1 });
await document.fonts.load("15px 'JetBrains Mono'"); // optional: the cell size is measured again when fonts finish loading
term.open(pane); // creates an element with a canvas, a scrollbar and a hidden textarea (it takes the focus)
term.fit(); // cols and rows from the size of `pane` (minus the scrollbar width, as xterm.js's FitAddon)
term.onCellSizeChange(() => term.fit()); // fonts loaded, devicePixelRatio changed
```

Drawing uses Canvas 2D with the browser's fonts (fallback fonts work for CJK), is crisp at any `devicePixelRatio`, and is batched with `requestAnimationFrame`: only changed rows are redrawn. `viewportY`, `scrollLines(n)`, `scrollToTop()`, `scrollToBottom()`, `scrollToLine(n)` and `onScroll` move the viewport into the scrollback; at the bottom it follows new output. Try it with `npm run build && node demo/serve.mjs` (open `/demo/`).

## Mouse and selection

`open()` binds the mouse for you. When the app asks for mouse reporting (modes 9, 1000, 1002, 1003 with the default, SGR 1006 or SGR-pixels 1016 encoding), presses, releases, motion and the wheel go out through `onData`, the same way as xterm.js 6.0.0 (the default encoding drops positions beyond column or row 223; 1005 and 1015 are treated as the default encoding, as xterm.js does not support them). Holding Shift forces selection instead. Without reporting, the wheel moves the viewport; on the alternate screen it sends Up/Down arrow keys. Drag selects a range, double click a word (`wordSeparator` option), triple click a line; wrapped lines are joined without a newline. `getSelection()`, `hasSelection()`, `getSelectionRange()`, `selectAll()`, `clearSelection()` and `onSelectionChange` are available, and the element answers the `copy` event with the selection (as xterm.js does). The selection color is `theme.selectionBackground`. Without a DOM, call `handleMouse()` and `handleWheel()` yourself.

## Keyboard, IME and paste

`open(host)` puts a hidden `textarea` in the element; `focus()` focuses it. Keys, IME text and pastes go out through `onData`, the same outlet as the replies. The bytes for keys are the same as xterm.js 6.0.0 (`evaluateKeyboardEvent` is exported and is a port of its `Keyboard.ts`):

- Enter, Backspace, Tab, Shift+Tab, Esc, arrows (`CSI` or `SS3` by DECCKM), Home, End, PageUp, PageDown, Insert, Delete, F1 to F12, modifiers (`CSI 1;m X`), Ctrl+letter, Alt as an ESC prefix. Shift+PageUp and Shift+PageDown scroll the viewport.
- On macOS, Option types a character (`macOptionIsMeta: false`, as xterm.js); set `macOptionIsMeta: true` to send ESC-prefixed keys. Keys with Cmd are not sent (left to the app's shortcuts).
- IME: nothing is sent while converting; the confirmed text is sent. The text being converted is shown at the cursor.
- `paste` events are handled; `term.paste(text)` does the same. Newlines become `\r`; with bracketed paste (2004) the text is wrapped in `ESC [200~` and `ESC [201~`.
- With focus reporting (1004), `CSI I` and `CSI O` are sent when the `textarea` gains and loses focus.
- With the kitty keyboard protocol on (flags set by the app), keys go out as `CSI ... u` as the spec says. `KeyHandler` also listens to `keyup` for flag 2: a release is sent only for a key whose press it sent as an escape code, so the Enter that ends an IME conversion does not produce a stray release. While converting, nothing changes: no escape codes, and the confirmed text is sent as plain text. Where the key is on the layout (flag 4) comes from `KeyboardEvent.code`, assuming a US PC-101 base layout.
- `attachCustomKeyEventHandler(fn)` sees key events first; return `false` to skip a key. `scrollOnUserInput` (default true) returns to the bottom when you type.

## Development

Everything runs in the Nix devShell (Rust with the `wasm32-unknown-unknown` target, wasm-bindgen, Node.js).

```sh
nix develop          # or: direnv allow
cargo test           # Rust tests
npm ci --ignore-scripts && npm run build && npm test   # WASM, package, TypeScript tests
nix fmt              # format
nix flake check      # formatting, cargo test, npm package build and tests (what CI runs)
nix build .#npm      # the npm tarball, with the .wasm and the .d.ts
```

See [AGENTS.md](AGENTS.md) for the rules. For publishing to crates.io and npm, see [RELEASING.md](RELEASING.md).

## License

[MIT](LICENSE)
