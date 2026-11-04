# coneko-term

**coneko-term は kitty とは別のプロジェクトです。** kitty の[プロトコル拡張](https://sw.kovidgoyal.net/kitty/protocol-extensions/)の 14 項目をすべて実装することを目指していますが、kitty が作ったものでも、kitty と関係があるものでもありません。

coneko-term は、ターミナルの部品のライブラリです。最初の使い手は [shijimi-editor](https://github.com/mm1995tk/shijimi-editor) の GUI で、今の xterm.js 6.0.0 と差し替えます。

- VT の処理と画面の中身（パーサー・セル・カーソル・スクロールバック）は Rust で持ち、WebAssembly にします。
- 描画は TypeScript の Canvas 2D です。字はブラウザーのフォントで描きます。
- Rust の本体はブラウザーに依存しません。あとでネイティブの GPU 描画（wgpu など）からも同じ本体を使えます。

最初のゴールは、xterm.js と入れ替えても shijimi-editor の GUI が今と同じように動くことです。kitty のプロトコルはそのあとに足します。「xterm.js と同じ」の物差しは、[xterm.js 6.0.0 の対応表](https://xtermjs.org/docs/api/vtfeatures/)と、そのソースです。

**状態:** WASM と TypeScript の API（`Terminal`）までできています。描画・キーボード・マウスはまだです。

## パッケージ

- crate `coneko-term`（コードでは `coneko_term`）: 本体。`crates/coneko-term` にあります。
- npm パッケージ `coneko-term`: WASM と TypeScript の型。ビルド済みの `.wasm` が入っているので、`npm --ignore-scripts` でも動きます。

```ts
import { init, Terminal } from "coneko-term";

await init();
const term = new Terminal({ cols: 80, rows: 24, scrollback: 1000 });
term.onData((bytes) => pty.write(bytes)); // 問い合わせへの返事（DSR など）と input() の出口
term.registerOscHandler(133, (data) => console.log("OSC 133", data));
term.write("hello\r\n", () => console.log("処理し終えた"));
```

`Terminal` は DOM に依存しません（`open(host)` はフォーカスを受ける要素を作るだけで、描画は描画の部品で足します）。大きな `write` は小分けにして処理し、そのデータを処理し終えたら `done` を呼びます。`takeDirtyRows()` と `readRow(y)` で、変わった行を WASM のメモリ上のセルとして読めます。

## 開発

すべて Nix の devShell（Rust と `wasm32-unknown-unknown` の target、wasm-bindgen、Node.js）の中で動かします。

```sh
nix develop          # または direnv allow
cargo test           # Rust のテスト
npm ci --ignore-scripts && npm run build && npm test   # WASM、パッケージ、TypeScript のテスト
nix fmt              # 整形
nix flake check      # 整形、cargo test、npm パッケージのビルドとテスト（CI が回すもの）
nix build .#npm      # .wasm と .d.ts を入れた npm の tarball
```

決まりは [AGENTS.md](AGENTS.md) にあります。

## ライセンス

[MIT](LICENSE)
