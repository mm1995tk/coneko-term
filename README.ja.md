# coneko-term

**coneko-term は kitty とは別のプロジェクトです。** kitty の[プロトコル拡張](https://sw.kovidgoyal.net/kitty/protocol-extensions/)の 14 項目をすべて実装することを目指していますが、kitty が作ったものでも、kitty と関係があるものでもありません。

coneko-term は、ターミナルの部品のライブラリです。最初の使い手は [shijimi-editor](https://github.com/mm1995tk/shijimi-editor) の GUI で、今の xterm.js 6.0.0 と差し替えます。

- VT の処理と画面の中身（パーサー・セル・カーソル・スクロールバック）は Rust で持ち、WebAssembly にします。
- 描画は TypeScript の Canvas 2D です。字はブラウザーのフォントで描きます。
- Rust の本体はブラウザーに依存しません。あとでネイティブの GPU 描画（wgpu など）からも同じ本体を使えます。

最初のゴールは、xterm.js と入れ替えても shijimi-editor の GUI が今と同じように動くことです。kitty のプロトコルはそのあとに足します。「xterm.js と同じ」の物差しは、[xterm.js 6.0.0 の対応表](https://xtermjs.org/docs/api/vtfeatures/)と、そのソースです。

**状態:** WASM と TypeScript の API（`Terminal`）と、Canvas 2D の描画（`fit()` とスクロールバーつき）までできています。マウスの報告・ホイール・選択とコピーまでできています。kitty keyboard protocol は一部できています。モードのフラグ（`CSI > u`、`CSI < u`、`CSI = u`、`CSI ? u`）と、エンコードのフラグ 5 つ（1 disambiguate escape codes、2 report event types、4 report alternate keys、8 report all keys as escape codes、16 report associated text）までできています。まだなのは、ロックの修飾キー（Caps Lock・Num Lock のビット）、Hyper と Meta、メディアキーです。コアは kitty graphics protocol で送られた画像（APC の `a=t`・`a=T`・`a=q`、分割送信、直接の送り方だけ）を受け取って持ち、セルに置き（`a=p`、`a=T`）、消し（`a=d`）、置いた配置を描く側へ一覧で渡せます。Canvas には描く側が描きます。`f=24`・`f=32`・`f=100` を、`o=z` ありでもなしでも描けます（PNG と zlib はブラウザーの `createImageBitmap` と `DecompressionStream` で読み解き、読み解けない画像は描きません）。`z` が 0 より小さいと字の下、-1073741824 より小さいとセルの背景色の下に描きます。Unicode placeholder（`U=1` の仮の配置と U+10EEEE、行と列の結合文字）も、結合文字を省いたときの推し量りを含めて描けます。アニメーションに対応しています。コアは、コマ（`a=f`。`x`・`y`・`s`・`v`、下地の `c`、編集する `r`、`X`、`Y`、表示時間 `z`）と、再生の状態（`a=a`。`s` で止める・待ちながら回す・回す、`c` で今のコマ、`v` で回数、`r` と `z` で表示時間）と、コマの合成（`a=c`）を持ちます。描画はブラウザーでコマを合成し、コマごとの表示時間のあとに `requestAnimationFrame` で切り替えます。コアは時計を持ちません。コマのデータも、画像と同じ持つ量の上限に数えます。まだないのは、コマだけを消すこと（`a=d` の `d=f`）です。デスクトップ通知（kitty の OSC 99）を読んでイベントにします（`onNotification`。`d=0` で分けて送られた通知は 1 つにそろえます）。返事（押された・閉じられた、`p=?` と `p=alive` への答え）は `Terminal` のメソッドで PTY へ書き戻します。OS に通知を出すのは使う側の仕事で、アイコンの画像は読みません。 kitty の clipboard protocol（OSC 5522）を読んでイベントにします。書き込みは MIME の種類・分割された base64・別名・主のクリップボードか primary selection を 1 つの `onClipboardWrite` にそろえ、読み取りの頼みは `onClipboardRead` になります。許すかどうかは使う側が決め、`replyClipboardWrite`・`replyClipboardReadTypes`・`replyClipboardReadData`・`replyClipboardReadError` で答えます。聞き手がいないときは、coneko-term が「許されない」（`EPERM`）で答えます。OS のクリップボードにつなぐのは使う側の仕事です。貼り付けイベント（`CSI ? 5522 h`）と `name`・`pw` の鍵は扱わず、OSC 52 もまだです。 kitty の drag and drop protocol（OSC 72）を読んでイベントにします（`onDnd`）。プログラムがドロップを受け付けたこと、ドラッグの途中の答え、落とされたデータの頼み、ドロップの終わり、プログラムが申し出たドラッグ（MIME の種類、先に送られたデータ、ドラッグの画像）が来ます。遠くの機械の形（`text/uri-list` の項目、シンボリックリンク、ディレクトリの handle）も扱います。OS のドロップは `dropMove`・`dropDrop`・`dropLeave` で渡し、データの頼みには `replyDropData` か `replyDropError` で答えます。ドラッグの身振りは `dragGesture`、始めたドラッグの進み具合は `dragStarted`・`dragError`・`dragAccepted`・`dragAction`・`dragDropped`・`dragFinished`・`requestDragData`・`requestDragEntry` で伝えます。エスケープシーケンス（分割、対応の問い合わせへの答え、ドロップの前や範囲外の頼みの拒否）は coneko-term が作ります。聞き手がいないときは「許されない」（`EPERM`）で答えます。OS のドラッグとドロップにつなぐこと、ファイルを読む・書くこと、機械の ID（HMAC-SHA256）を作って比べることは使う側の仕事です。kitty の text sizing protocol（OSC 66）に対応しています。`s`・`w`・`n`/`d`・`v`/`h` を読み、複数のセルにまたがる字（幅 `w * s`、高さ `s`。カーソルの位置から右と下へ広がる）を画面のモデルに持ち、Canvas に大きさと寄せ方のとおり描きます。字の一部を上書き・消去すると、仕様のとおり字全体が消えます。範囲外の値、4096 バイトを超える字、制御文字を含む字は、シーケンス全体を捨てます。列の数が変わると、この字はふつうの大きさの字に戻ります。TypeScript へ渡すセルは 1 つ 7 個の値になり（`CELL_STRIDE`）、増えた 1 つがこの字のセルの印です（`RowView.sized`）。置いたあとの字に結合文字を足すことはまだで、捨てます。

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

`Terminal` は `open(host)` を呼ぶまで DOM に依存しません。大きな `write` は小分けにして処理し、そのデータを処理し終えたら `done` を呼びます。`takeDirtyRows()` と `readRow(y)` で、変わった行を WASM のメモリ上のセルとして読めます。

## 描画

```ts
const term = new Terminal({ fontFamily: "'JetBrains Mono', monospace", fontSize: 15, lineHeight: 1 });
await document.fonts.load("15px 'JetBrains Mono'"); // 省いてもよい: フォントを読み終えたときにセルの大きさを測り直す
term.open(pane); // canvas・スクロールバー・見えない textarea（フォーカスを受ける）を入れた要素を作る
term.fit(); // `pane` の大きさから列と行の数を出す（xterm.js の FitAddon と同じく、スクロールバーの幅を引く）
term.onCellSizeChange(() => term.fit()); // フォントを読み終えた、devicePixelRatio が変わった
```

描画は Canvas 2D で、字はブラウザーのフォントで描きます（日本語は fallback で描けます）。`devicePixelRatio` が高くてもにじまず、`requestAnimationFrame` でまとめて、変わった行だけを描き直します。`viewportY`、`scrollLines(n)`、`scrollToTop()`、`scrollToBottom()`、`scrollToLine(n)`、`onScroll` でスクロールバックを見る位置を動かせます。一番下を見ているときは新しい出力に付いていきます。`npm run build && node demo/serve.mjs` のあと `/demo/` を開くと試せます。

## マウスと選択

`open()` がマウスを受けます。アプリがマウスの報告（モード 9、1000、1002、1003。書き方は既定・SGR 1006・ピクセル単位の SGR 1016）を頼むと、押した・離した・動かした・ホイールを `onData` から出します（xterm.js 6.0.0 と同じ。既定の書き方では、列か行が 223 を超える位置は送りません。1005 と 1015 は xterm.js が対応しないので既定の書き方として扱います）。Shift を押していれば、報告せずに選択します。報告が無効なら、ホイールは表示位置を動かし、代替画面では上下の矢印キーを送ります。ドラッグで範囲、ダブルクリックで単語（`wordSeparator` で区切りを変えられます）、トリプルクリックで行を選び、折り返した行は改行を入れずにつなぎます。`getSelection()`、`hasSelection()`、`getSelectionRange()`、`selectAll()`、`clearSelection()`、`onSelectionChange` が使え、要素は `copy` イベントに選択の字を渡します（xterm.js と同じ）。選択の色は `theme.selectionBackground` です。DOM が無いときは `handleMouse()` と `handleWheel()` を自分で呼びます。

## キーボード・IME・貼り付け

`open(host)` は、要素の中に見えない `textarea` を置きます。`focus()` はこれにフォーカスします。キー・IME の字・貼り付けは、問い合わせへの返事と同じ出口（`onData`）から出ます。キーのバイト列は xterm.js 6.0.0 と同じです（`evaluateKeyboardEvent` は `Keyboard.ts` の移植で、外にも出しています）。

- Enter、Backspace、Tab、Shift+Tab、Esc、矢印（DECCKM で `CSI` と `SS3` を切り替える）、Home、End、PageUp、PageDown、Insert、Delete、F1〜F12、修飾キー付き（`CSI 1;m X`）、Ctrl+字、Alt を ESC の前置きにする。Shift+PageUp・Shift+PageDown は表示位置を動かします。
- macOS の Option は字を打つキーとして扱います（`macOptionIsMeta: false`。xterm.js と同じ）。`true` にすると ESC の前置きにします。Cmd を押したキーは送りません（アプリのショートカットに任せる）。
- IME: 変換している間は送らず、確定した字を送ります。変換中の字はカーソルの位置に見せます。
- `paste` イベントを受けます。`term.paste(text)` でも同じことができます。改行は `\r` にそろえ、bracketed paste（2004）が有効なら `ESC [200~` と `ESC [201~` で囲みます。
- フォーカスの報告（1004）が有効なら、`textarea` がフォーカスを得たときと失ったときに `CSI I` と `CSI O` を送ります。
- kitty keyboard protocol が有効なとき（アプリがフラグを立てたとき）は、仕様どおり `CSI ... u` で送ります。フラグ 2 のため `KeyHandler` は `keyup` も受けますが、離すを送るのは、押したのをエスケープコードで送ったキーだけです（IME の変換を確定する Enter が、押していないのに離すだけ送られることはありません）。変換中の動きは変わらず、エスケープコードにせず、確定した字はそのままの文字で送ります。ベースのレイアウトのキー（フラグ 4）は `KeyboardEvent.code` から決めます（PC-101 の US 配列を基準にします）。
- `attachCustomKeyEventHandler(fn)` はキーのイベントを先に見ます。`false` を返したキーは処理しません。`scrollOnUserInput`（既定は true）は、入力したら一番下へ戻します。

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

決まりは [AGENTS.md](AGENTS.md) にあります。crates.io と npm への公開は [RELEASING.md](RELEASING.md) を見てください。

## ライセンス

[MIT](LICENSE)
