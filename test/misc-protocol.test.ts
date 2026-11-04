import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { before, test } from "node:test";
import { init, Terminal, type ProgramStatusChange } from "../dist/index.js";

// kitty の miscellaneous protocol extensions。
// 仕様: https://sw.kovidgoyal.net/kitty/misc-protocol/

before(async () => {
  await init(
    await readFile(
      new URL("../dist/wasm/coneko_term_wasm_bg.wasm", import.meta.url),
    ),
  );
});

function write(t: Terminal, data: string | Uint8Array): Promise<void> {
  return new Promise((resolve) => t.write(data, resolve));
}

function collect(t: Terminal): { out: string[] } {
  const box = { out: [] as string[] };
  t.onData((d) => box.out.push(String.fromCharCode(...d)));
  return box;
}

test("save and restore all modes (CSI ? s, CSI ? r)", async () => {
  const t = new Terminal({ cols: 20, rows: 5 });
  const box = collect(t);
  await write(t, "\x1b[?1000h\x1b[?1006h\x1b[?s");
  await write(t, "\x1b[?1000l\x1b[?1006l");
  assert.equal(t.mouseReporting, false);
  await write(t, "\x1b[?r");
  assert.equal(t.mouseReporting, true);
  assert.deepEqual(box.out, []);
  t.dispose();
});

test("CSI 22 J moves the screen into the scrollback", async () => {
  const t = new Terminal({ cols: 10, rows: 4 });
  await write(t, "one\r\ntwo");
  assert.equal(t.scrollbackLength, 0);
  await write(t, "\x1b[22J");
  assert.equal(t.scrollbackLength, 2);
  assert.equal(t.getRowText(0).trim(), "");
  t.dispose();
});

test("SGR 221 and 222 reset bold and faint separately", async () => {
  const t = new Terminal({ cols: 10, rows: 2 });
  await write(t, "\x1b[1;2m\x1b[221mA\x1b[1;2m\x1b[222mB\x1b[1;2m\x1b[22mC");
  const attrs = (x: number) => t.readRow(0).cells[x * 7 + 4];
  // 太字だけ消えた A と、薄字だけ消えた B と、両方消えた C は、どれも属性が違う
  assert.notEqual(attrs(0), attrs(1));
  assert.notEqual(attrs(0), attrs(2));
  assert.notEqual(attrs(1), attrs(2));
  t.dispose();
});

test("program status (OSC 7501) becomes events and records", async () => {
  const t = new Terminal({ cols: 20, rows: 3 });
  const changes: ProgramStatusChange[] = [];
  t.onProgramStatus((c) => changes.push(c));
  const box = collect(t);
  await write(t, "\x1b]7501;state=working:app=rsync:progress=40\x1b\\");
  assert.equal(changes.length, 1);
  assert.deepEqual(changes[0], {
    id: "",
    status: { id: "", state: "working", progress: 40, app: "rsync" },
  });
  assert.deepEqual(t.programStatus, [changes[0].status]);

  // 仕様の例（詰め物つきの base64）
  await write(
    t,
    "\x1b]7501;state=blocked:kind=permission:app=terraform:msg=QXBwbHkgMyB0byBhZGQsIDEgdG8gY2hhbmdlLCAwIHRvIGRlc3Ryb3k/\x1b\\",
  );
  assert.equal(
    t.programStatus[0].msg,
    "Apply 3 to add, 1 to change, 0 to destroy?",
  );
  assert.equal(t.programStatus[0].kind, "permission");

  // done は、フォーカスで消える
  await write(t, "\x1b]7501;state=done:id=job\x1b\\");
  assert.equal(t.programStatus.length, 2);
  changes.length = 0;
  t.setFocused(true);
  assert.deepEqual(changes, [{ id: "job" }]);
  assert.equal(t.programStatus.length, 1);

  // プロセスの終了で、blocked が消える
  t.programStatusProcessExited();
  assert.deepEqual(t.programStatus, []);

  // 問い合わせ
  await write(t, "\x1b]7501;?\x1b\\");
  assert.deepEqual(box.out, [
    "\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x1b\\",
  ]);
  t.dispose();
});

test("program status: clear and RIS remove records", async () => {
  const t = new Terminal({ cols: 20, rows: 3 });
  const changes: ProgramStatusChange[] = [];
  t.onProgramStatus((c) => changes.push(c));
  await write(t, "\x1b]7501;state=working:id=a\x1b\\\x1b]7501;state=working:id=a/b\x1b\\");
  changes.length = 0;
  await write(t, "\x1b]7501;state=clear:id=a\x1b\\");
  assert.deepEqual(changes, [{ id: "a" }, { id: "a/b" }]);
  await write(t, "\x1b]7501;state=working\x1b\\");
  changes.length = 0;
  await write(t, "\x1bc");
  assert.deepEqual(changes, [{ id: "" }]);
  assert.deepEqual(t.programStatus, []);
  t.dispose();
});
