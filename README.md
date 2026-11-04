# coneko-term

**coneko-term is a separate project from kitty.** It aims to implement all 14 items of kitty's [protocol extensions](https://sw.kovidgoyal.net/kitty/protocol-extensions/), but it is not made by or affiliated with kitty.

coneko-term is a library of terminal building blocks. The first user is the GUI of [shijimi-editor](https://github.com/mm1995tk/shijimi-editor), where it replaces xterm.js 6.0.0.

- The VT processing and the screen (parser, cells, cursor, scrollback) are written in Rust and compiled to WebAssembly.
- Drawing is TypeScript on Canvas 2D. Glyphs are drawn with the browser's fonts.
- The Rust core does not depend on the browser, so a native GPU renderer (wgpu and so on) can use the same core later.

The first goal is that shijimi-editor's GUI works with coneko-term as it does with xterm.js. kitty's protocol comes after that. "The same as xterm.js" is measured against the [xterm.js 6.0.0 feature table](https://xtermjs.org/docs/api/vtfeatures/) and its source.

**Status:** the foundation only. The API has one function, `version()`.

## Packages

- crate `coneko-term` (`coneko_term` in code): the core, in `crates/coneko-term`.
- npm package `coneko-term`: the WASM and the TypeScript types. The built `.wasm` is in the package, so it works with `npm --ignore-scripts`.

```ts
import { init, version } from "coneko-term";

await init();
console.log(version());
```

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

See [AGENTS.md](AGENTS.md) for the rules.

## License

[MIT](LICENSE)
