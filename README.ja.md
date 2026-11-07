# coneko-term

**coneko-term は kitty とは別のプロジェクトです。** kitty の[プロトコル拡張](https://sw.kovidgoyal.net/kitty/protocol-extensions/)の 14 項目をすべて実装することを目指していますが、kitty が作ったものでも、kitty と関係があるものでもありません。

coneko-term は、ターミナルの部品のライブラリです。最初の使い手は [shijimi-editor](https://github.com/mm1995tk/shijimi-editor) の GUI で、今の xterm.js 6.0.0 と差し替えます。

- VT の処理と画面の中身（パーサー・セル・カーソル・スクロールバック）は Rust で持ち、WebAssembly にします。
- 描画は TypeScript の Canvas 2D です。字はブラウザーのフォントで描きます。
- Rust の本体はブラウザーに依存しません。あとでネイティブの GPU 描画（wgpu など）からも同じ本体を使えます。

最初のゴールは、xterm.js と入れ替えても shijimi-editor の GUI が今と同じように動くことです。kitty のプロトコルはそのあとに足します。「xterm.js と同じ」の物差しは、[xterm.js 6.0.0 の対応表](https://xtermjs.org/docs/api/vtfeatures/)と、そのソースです。

**状態:** WASM と TypeScript の API（`Terminal`）と、Canvas 2D の描画（`fit()` とスクロールバーつき）までできています。マウスの報告・ホイール・選択とコピーまでできています。kitty keyboard protocol は一部できています。モードのフラグ（`CSI > u`、`CSI < u`、`CSI = u`、`CSI ? u`）と、エンコードのフラグ 5 つ（1 disambiguate escape codes、2 report event types、4 report alternate keys、8 report all keys as escape codes、16 report associated text）までできています。まだなのは、ロックの修飾キー（Caps Lock・Num Lock のビット）、Hyper と Meta、メディアキーです。コアは kitty graphics protocol で送られた画像（APC の `a=t`・`a=T`・`a=q`、分割送信、直接の送り方だけ）を受け取って持ち、セルに置き（`a=p`、`a=T`）、消し（`a=d`）、置いた配置を描く側へ一覧で渡せます。Canvas には描く側が描きます。`f=24`・`f=32`・`f=100` を、`o=z` ありでもなしでも描けます（PNG と zlib はブラウザーの `createImageBitmap` と `DecompressionStream` で読み解き、読み解けない画像は描きません）。`z` が 0 より小さいと字の下、-1073741824 より小さいとセルの背景色の下に描きます。Unicode placeholder（`U=1` の仮の配置と U+10EEEE、行と列の結合文字）も、結合文字を省いたときの推し量りを含めて描けます。アニメーションに対応しています。コアは、コマ（`a=f`。`x`・`y`・`s`・`v`、下地の `c`、編集する `r`、`X`、`Y`、表示時間 `z`）と、再生の状態（`a=a`。`s` で止める・待ちながら回す・回す、`c` で今のコマ、`v` で回数、`r` と `z` で表示時間）と、コマの合成（`a=c`）を持ちます。描画はブラウザーでコマを合成し、コマごとの表示時間のあとに `requestAnimationFrame` で切り替えます。コアは時計を持ちません。コマのデータも、画像と同じ持つ量の上限に数えます。まだないのは、コマだけを消すこと（`a=d` の `d=f`）です。デスクトップ通知（kitty の OSC 99）を読んでイベントにします（`onNotification`。`d=0` で分けて送られた通知は 1 つにそろえます）。返事（押された・閉じられた、`p=?` と `p=alive` への答え）は `Terminal` のメソッドで PTY へ書き戻します。OS に通知を出すのは使う側の仕事で、アイコンの画像は読みません。 kitty の clipboard protocol（OSC 5522）を読んでイベントにします。書き込みは MIME の種類・分割された base64・別名・主のクリップボードか primary selection を 1 つの `onClipboardWrite` にそろえ、読み取りの頼みは `onClipboardRead` になります。許すかどうかは使う側が決め、`replyClipboardWrite`・`replyClipboardReadTypes`・`replyClipboardReadData`・`replyClipboardReadError` で答えます。聞き手がいないときは、coneko-term が「許されない」（`EPERM`）で答えます。OS のクリップボードにつなぐのは使う側の仕事です。貼り付けイベント（`CSI ? 5522 h`）と `name`・`pw` の鍵は扱わず、OSC 52 もまだです。 kitty の drag and drop protocol（OSC 72）を読んでイベントにします（`onDnd`）。プログラムがドロップを受け付けたこと、ドラッグの途中の答え、落とされたデータの頼み、ドロップの終わり、プログラムが申し出たドラッグ（MIME の種類、先に送られたデータ、ドラッグの画像）が来ます。遠くの機械の形（`text/uri-list` の項目、シンボリックリンク、ディレクトリの handle）も扱います。OS のドロップは `dropMove`・`dropDrop`・`dropLeave` で渡し、データの頼みには `replyDropData` か `replyDropError` で答えます。ドラッグの身振りは `dragGesture`、始めたドラッグの進み具合は `dragStarted`・`dragError`・`dragAccepted`・`dragAction`・`dragDropped`・`dragFinished`・`requestDragData`・`requestDragEntry` で伝えます。エスケープシーケンス（分割、対応の問い合わせへの答え、ドロップの前や範囲外の頼みの拒否）は coneko-term が作ります。聞き手がいないときは「許されない」（`EPERM`）で答えます。OS のドラッグとドロップにつなぐこと、ファイルを読む・書くこと、機械の ID（HMAC-SHA256）を作って比べることは使う側の仕事です。kitty の text sizing protocol（OSC 66）に対応しています。`s`・`w`・`n`/`d`・`v`/`h` を読み、複数のセルにまたがる字（幅 `w * s`、高さ `s`。カーソルの位置から右と下へ広がる）を画面のモデルに持ち、Canvas に大きさと寄せ方のとおり描きます。字の一部を上書き・消去すると、仕様のとおり字全体が消えます。範囲外の値、4096 バイトを超える字、制御文字を含む字は、シーケンス全体を捨てます。列の数が変わると、この字はふつうの大きさの字に戻ります。TypeScript へ渡すセルは 1 つ 7 個の値になり（`CELL_STRIDE`）、増えた 1 つがこの字のセルの印です（`RowView.sized`）。置いたあとの字に結合文字を足すことはまだで、捨てます。 kitty の file transfer protocol（OSC 5113）を読んでイベントにします（`onFileTransfer`）。セッションの始まり（`send`・`receive`）、ファイルの情報とパスの頼み、分割されたデータ、圧縮の印（`zip=zlib`）、終わり、取り消しが来ます。許すかどうかは使う側が決め、`replyFileTransferOk`・`replyFileTransferStarted`・`replyFileTransferProgress`・`replyFileTransferError`・`refuseFileTransfer`・`replyFileTransferCanceled` で答え、受け取りの一覧とデータは `replyFileTransferListing` と `replyFileTransferData` で送ります。答えのエスケープシーケンス（4096 バイトごとの分割、`q` による省略）は coneko-term が作ります。聞き手がいないときは、`send` と `receive` を「許されない」（`EPERM`）で断り、取り消しには `CANCELED` で答えます。ファイルを読む・書くこと、パスの検査、圧縮と rsync の署名・差分を作る・読み解くこと、`pw` の照らし合わせは使う側の仕事です。kitty の multiple cursors protocol（`CSI > ... SP q`）に対応しています。追加のカーソルを足す・消す・問い合わせることができ（位置は 1 つずつ、四角、主のカーソルの位置で指定）、形（ブロック・縦棒・下線・主のカーソルに従う）と、追加のカーソルが共有する色の組（カーソルの色と字の色。`CSI > 30`、`CSI > 40`）も決められ、Canvas の描画が描きます。追加のカーソルは主のカーソルとは別に画面のモデルが持ちます（`Screen::extra_cursors`、`Terminal.extraCursors`）。`CSI 2 J`、`CSI 3 J`、`CSI 22 J`、RIS、主画面と代替画面の切り替えで消え、スクロールでは動きません。大きさが変わると、新しい画面の外に出たものを捨てます。点滅は主のカーソルと同じで、主のカーソルが見えなくても出ます。 kitty の mouse pointer shapes（OSC 22）に対応しています。設定・積む・降ろす・問い合わせ（`?`。`__current__`・`__default__`・`__grabbed__` を含む）を仕様どおりに扱い、主画面と代替画面で別々の積み重ねを持ちます。形の名前は CSS の `cursor` の値です。`Terminal.pointerShape` が今の形を返し、`open` で作った要素の `cursor` にも反映します。マウスの報告が有効でも無効でも効きます。まだ扱わないもの: xterm の古い形の名前（別名）。kitty の unscrolling（`CSI n + T`）に対応しています。画面を n 行下へずらし、上にできた空きにスクロールバックの最後の行を属性ごと戻します。スクロールバックに無い分は空の行です。スクロールバックの無い代替画面では `CSI n T` と同じです。スクロール領域があるときは領域の中で動き、行は主画面のスクロールバックから戻ります。 kitty の矩形への属性の指定（DECCARA・DECRARA）に対応しています。`CSI Pt;Pl;Pb;Pr;Ps$ r` で属性を付け、`CSI Pt;Pl;Pb;Pr;Ps$ t` で反転します。kitty の拡張のとおり、DECCARA には SGR の全部（256 色・RGB の色、下線の形と色を含む）を指定できます。DECRARA は太字・薄字・斜体・下線・点滅・反転・不可視・取り消し線を反転し、色は反転しません（`0` と省略は DEC の決まりの太字・下線・点滅・反転）。DECSACE（`CSI Ps * x`）で範囲を選びます。`2` が矩形、`0`・`1` と省略が行の流れに沿った範囲（既定）です。原点モード（DECOM）ではスクロール範囲の左上から数え、範囲の外に出る下端は範囲の最後の行に収めます。左右の余白（DECLRMM）にはまだ対応していません。 kitty の color control（OSC 21）に対応しています。仕様のキー（`foreground`・`background`・`selection_background`・`selection_foreground`・`cursor`・`cursor_text`・`visual_bell`・`transparent_background_color1` から `7`・色の表の 0 から 255）すべてについて、読む（`key=?`）・書く（`key=色`）・動的にする（`key=`）・既定に戻す（キーだけ）ができ、答えは仕様の形です（`rgb:rr/gg/bb`。決まっていない色は空、知らないキーは `unknown=` と = なしの base64）。色の書き方は `rgb:`・`#`・`rgbi:`・名前で、`@` の後ろの不透明度は読んで捨て、名前は X11 の色の一部（red・green・blue など少数）だけ知っています。読めない値は捨てます。OSC 4・10・11・12 でも色を書けるようになり（戻すのは OSC 104・110・111・112）、OSC 21 と同じ色の表を使うので、どちらで書いてもどちらからも読めます。描画は、変わった文字・背景・カーソルの色、カーソルの下の字の色、選択範囲の背景、色の表に効き、`Terminal` は `write` のたびに変化を確かめます。コアは `Terminal::colors` で渡します。描かないもの: `selection_foreground`・`visual_bell`・透明な背景色（持っていて読めます）。選択範囲の背景は、字が見えるように `theme.selectionBackground` と同じ透明度で描きます（OSC 21 で決めた色は `4d`）。色の積み重ねにも対応しています。`CSI # P`（XTPUSHCOLORS）・`CSI # Q`（XTPOPCOLORS）・`CSI # R`（XTREPORTCOLORS。`CSI Pi ; Pn # Q` で答える）で 10 段、OSC 30001 と OSC 30101 もあります。RIS で色をテーマに戻し、積み重ねを空にします。 kitty の miscellaneous protocol の小さな拡張に対応しています。`CSI ? s` と `CSI ? r` は端末のモードの保存と復元です。モードを並べたとき（XTSAVE と XTRESTORE）はそのモードだけ、並べないとき（kitty の拡張）は副作用の無いすべてのモード（DECOM と、画面を切り替えるモードは入りません）です。`CSI 221 m` と `CSI 222 m` は太字と薄字を別々に消します（`CSI 22 m` は今までどおり両方）。マウスの報告が `CSI ? 1003 h` とピクセル単位の SGR（`CSI ? 1016 h`）のとき、`open` が作る要素は、マウスが窓から出たことを `CSI < 163 ; 0 ; 0 M` で報告します（`Terminal.handleMouse` の `type: "leave"`。ボタンを押している間は報告しません）。`CSI 22 J` は画面の字と画像をスクロールバックへ移し、画面を `CSI 2 J` のあとと同じにします。何かある最後の行までを移すので空の行は積まれず、代替画面では消すだけです。Program Status Protocol（OSC 7501、改訂 0.4）を読んでイベント（`onProgramStatus`）にし、`Terminal.programStatus` で記録の一覧を返します。状態（idle・working・done・blocked・error）、`kind`、`progress`、`app`（いちばん近い親から引き継ぎ）、`title`、`msg`、id と `state=clear`、上限（超えた報告は丸ごと捨てます）、機能の問い合わせへの返事に対応しています。記録は、RIS、`state=clear`、新しいシェルのプロンプト（OSC 133 の `A`。working・blocked・idle）、窓がフォーカスを得たときとキーを押したとき（done・error）、アプリケーションが `programStatusProcessExited` を呼んだとき（working・blocked・idle）に消えます。記録は表示を決めません。何を見せるかはアプリケーションの仕事です。壊れた `id` の報告は、根の記録を書き換えないよう丸ごと無視します。未対応: OSC 9;4 を記録へ割り当てること。「kitty specific private escape codes」の節は扱いません。

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

## 画面の状態の書き出し（Rust の crate）

crate の `Terminal::serialize(max_scrollback)` は、同じ大きさの新しい `Terminal` に書けば同じ画面になるバイト列を返します。xterm.js の `SerializeAddon` と同じ考え方です。サーバーが画面を持たない `Terminal` を動かしておき、あとから繋いだクライアントには、過去の出力を流し直す代わりにこの状態を渡せます（流し直すと、DECRQM などの昔の問い合わせにもう一度返事をしてしまい、その返事がシェルに入力として届きます）。

```rust
let bytes = term.serialize(5000); // scrollback は新しいほうから 5000 行
let mut other = coneko_term::Terminal::new(rows, cols);
other.write(&bytes);
assert!(other.take_output().is_empty()); // 問い合わせは入っていない
```

含むもの: 見えている画面と、主画面の scrollback（新しいほうから `max_scrollback` 行。文字・SGR の属性・折り返した行のつながり）、代替画面を出しているか（主画面も一緒に書く）、カーソル（位置・折り返し待ち・形・点滅・表示）、DECSC で保存したカーソル、スクロール範囲、タブ位置、主画面と代替画面それぞれの kitty keyboard のフラグの積み重ね、DEC モード（マウス報告・bracketed paste・フォーカスの報告・DECCKM・DECOM・自動折り返しなど）、IRM、LNM、`Theme` から変わっている文字・背景・カーソルの色とパレット（書く側と書かれる側のテーマは同じにしてください）。

含まないもの: kitty graphics の画像とその配置、文字集合（G0〜G3）、`CSI ? s` で保存したモード、追加のカーソル、色の積み重ね、OSC 21 だけで変わる色、題名、通知・クリップボード・drag and drop・file transfer の途中の状態。scrollback の最後の行の「折り返している」印は残らず、scrollback の中では 1 行より高い OSC 66 の文字は書きません。npm 版にはまだ出していません。

## 描画

```ts
const term = new Terminal({ fontFamily: "'JetBrains Mono', monospace", fontSize: 15, lineHeight: 1 });
await document.fonts.load("15px 'JetBrains Mono'"); // 省いてもよい: フォントを読み終えたときにセルの大きさを測り直す
term.open(pane); // canvas・スクロールバー・見えない textarea（フォーカスを受ける）を入れた要素を作る
term.fit(); // `pane` の大きさから列と行の数を出す（xterm.js の FitAddon と同じく、スクロールバーの幅を引く）
term.onCellSizeChange(() => term.fit()); // フォントを読み終えた、devicePixelRatio が変わった
```

スクロールバーの幅は既定で 14 CSS ピクセルです。`new Terminal({ ... })` に `scrollbarWidth` を渡すと変えられます（例: `scrollbarWidth: 8`）。`fit()` はその幅を引きます。`scrollbarWidth: 0` ならスクロールバーを出さず、何も引きません。負の数や数でない値は 14 に戻ります。

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
