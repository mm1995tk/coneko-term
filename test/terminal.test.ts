import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { before, test } from "node:test";
import { init, Terminal, type FileTransferEvent } from "../dist/index.js";

before(async () => {
  await init(
    await readFile(
      new URL("../dist/wasm/coneko_term_wasm_bg.wasm", import.meta.url),
    ),
  );
});

/** `write` して、処理し終えるまで待つ。 */
function write(t: Terminal, data: string | Uint8Array): Promise<void> {
  return new Promise((resolve) => t.write(data, resolve));
}

const dec = new TextDecoder();

test("write reaches the screen", async () => {
  const t = new Terminal({ cols: 10, rows: 3 });
  await write(t, "hello\r\nworld");
  assert.equal(t.getRowText(0), "hello     ");
  assert.equal(t.getRowText(1), "world     ");
  assert.deepEqual(
    { x: t.cursor.x, y: t.cursor.y },
    { x: 5, y: 1 },
  );
  assert.equal(t.cols, 10);
  assert.equal(t.rows, 3);
  t.dispose();
});

test("write accepts bytes, and a character split across writes", async () => {
  const t = new Terminal({ cols: 4, rows: 1 });
  const bytes = new TextEncoder().encode("あ");
  t.write(bytes.subarray(0, 1));
  await write(t, bytes.subarray(1));
  assert.equal(t.getRowText(0), "あ  ");
  const row = t.readRow(0);
  assert.equal(row.cells[0], "あ".codePointAt(0));
  assert.equal(row.cells[1], 2);
  t.dispose();
});

test("done is called after the data is processed, in order", async () => {
  const t = new Terminal({ cols: 10, rows: 2 });
  const seen: string[] = [];
  const p1 = new Promise<void>((r) =>
    t.write("one", () => {
      seen.push("1:" + t.getRowText(0).trim());
      r();
    }),
  );
  const p2 = new Promise<void>((r) =>
    t.write("two", () => {
      seen.push("2:" + t.getRowText(0).trim());
      r();
    }),
  );
  assert.deepEqual(seen, []);
  await Promise.all([p1, p2]);
  assert.deepEqual(seen, ["1:one", "2:onetwo"]);
  t.dispose();
});

test("a large write is split so other tasks can run", async () => {
  const t = new Terminal({ cols: 80, rows: 24, scrollback: 100 });
  const line = "x".repeat(79) + "\r\n";
  const data = line.repeat(60_000); // 約 4.8 MB
  let finished = false;
  let ticks = 0;
  const done = new Promise<void>((r) =>
    t.write(data, () => {
      finished = true;
      r();
    }),
  );
  const timer = setInterval(() => {
    if (!finished) ticks++;
  }, 0);
  await done;
  clearInterval(timer);
  assert.ok(ticks > 0, "timers should run while the write is being processed");
  assert.equal(t.getRowText(0), "x".repeat(79) + " ");
  t.dispose();
});

test("DSR 6 is answered through onData", async () => {
  const t = new Terminal({ cols: 10, rows: 5 });
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  await write(t, "\r\n\r\nab\x1b[6n");
  assert.deepEqual(out, ["\x1b[3;3R"]);
  t.dispose();
});

test("DA1 and color queries use the theme", async () => {
  const t = new Terminal({ theme: { background: "#102030" } });
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  await write(t, "\x1b]11;?\x07");
  assert.deepEqual(out, ["\x1b]11;rgb:1010/2020/3030\x07"]);
  t.dispose();
});

test("OSC 21 changes the colors the renderer uses, and OSC 4/10/11 read them", async () => {
  const t = new Terminal();
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  const seen: unknown[] = [];
  (t as unknown as { renderer: unknown }).renderer = {
    setOptions: (o: unknown) => seen.push(o),
    touch: () => {},
  };
  await write(t, "\x1b]21;background=#102030;1=#ff8000\x07\x1b]10;?\x07\x1b]11;?\x07\x1b]4;1;?\x07");
  assert.equal(
    out.join(""),
    "\x1b]10;rgb:ffff/ffff/ffff\x07\x1b]11;rgb:1010/2020/3030\x07\x1b]4;1;rgb:ffff/8080/0000\x07",
  );
  assert.equal(seen.length, 1);
  const palette = (seen[0] as { palette: { background: string; get(i: number): string } }).palette;
  assert.equal(palette.background, "#102030");
  assert.equal(palette.get(1), "#ff8000");
  out.length = 0;
  await write(t, "\x1b]21;background\x07\x1b]11;?\x07");
  assert.deepEqual(out, ["\x1b]11;rgb:0000/0000/0000\x07"]);
  assert.equal(seen.length, 2);
});

test("input goes out through the same onData", () => {
  const t = new Terminal();
  const out: Uint8Array[] = [];
  t.onData((d) => out.push(d));
  t.input("ls\r");
  t.input(Uint8Array.of(1, 2, 3));
  assert.deepEqual(out.map((d) => [...d]), [[108, 115, 13], [1, 2, 3]]);
  t.dispose();
});

test("OSC handlers get registered numbers only", async () => {
  const t = new Terminal();
  const got: string[] = [];
  const titles: string[] = [];
  t.onTitleChange((s) => titles.push(s));
  const d = t.registerOscHandler(133, (s) => got.push(s));
  await write(t, "\x1b]133;A;日本\x07\x1b]2;title\x07\x1b]999;x\x07");
  assert.deepEqual(got, ["A;日本"]);
  assert.deepEqual(titles, ["title"]);
  d.dispose();
  await write(t, "\x1b]133;B\x07");
  assert.deepEqual(got, ["A;日本"]);
  t.dispose();
});

test("bell and focus reports", async () => {
  const t = new Terminal();
  let bells = 0;
  t.onBell(() => bells++);
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  await write(t, "\x07");
  assert.equal(bells, 1);
  t.setFocused(true);
  assert.deepEqual(out, []);
  await write(t, "\x1b[?1004h");
  t.setFocused(false);
  assert.deepEqual(out, ["\x1b[O"]);
  t.dispose();
});

test("dirty rows are reported once, and cells carry attributes", async () => {
  const t = new Terminal({ cols: 5, rows: 3 });
  t.takeDirtyRows();
  await write(t, "\x1b[2;1H\x1b[1;38;2;1;2;3mA\x1b[0mé");
  assert.deepEqual(t.takeDirtyRows(), [1]);
  assert.deepEqual(t.takeDirtyRows(), []);
  const row = t.readRow(1);
  assert.equal(row.cells[0], "A".codePointAt(0));
  assert.equal(row.cells[2], (2 << 24) | 0x010203);
  assert.equal(row.cells[4] & 1, 1);
  assert.equal(t.getRowText(1), "Aé   ");
  t.dispose();
});

test("scrollback, reset and resize", async () => {
  const t = new Terminal({ cols: 5, rows: 2, scrollback: 10 });
  await write(t, "a\r\nb\r\nc");
  assert.equal(t.scrollbackLength, 1);
  assert.equal(dec.decode(Uint8Array.of(t.readScrollbackRow(0).cells[0])), "a");
  t.resize(8, 4);
  assert.equal(t.cols, 8);
  assert.equal(t.rows, 4);
  t.reset();
  assert.equal(t.getRowText(0).trim(), "");
  t.dispose();
});

test("write after dispose is ignored", async () => {
  const t = new Terminal();
  t.dispose();
  let called = false;
  t.write("x", () => (called = true));
  await new Promise((r) => setTimeout(r, 5));
  assert.equal(called, false);
});

test("an exception in done does not stop later writes", async () => {
  const t = new Terminal({ cols: 4, rows: 1 });
  const errors: unknown[] = [];
  const orig = globalThis.queueMicrotask;
  globalThis.queueMicrotask = (f) =>
    orig(() => {
      try {
        f();
      } catch (e) {
        errors.push(e);
      }
    });
  t.write("a", () => {
    throw new Error("boom");
  });
  await write(t, "b");
  await new Promise((r) => setTimeout(r, 5));
  assert.equal(t.getRowText(0), "ab  ");
  assert.equal(errors.length, 1);
  globalThis.queueMicrotask = orig;
  t.dispose();
});

test("write copies the buffer, and a disposed terminal throws clearly", async () => {
  const t = new Terminal({ cols: 4, rows: 1 });
  const buf = new TextEncoder().encode("ab");
  const p = write(t, buf);
  buf.fill(0x7a);
  await p;
  assert.equal(t.getRowText(0), "ab  ");
  t.dispose();
  assert.throws(() => t.readRow(0), /disposed/);
});

test("notifications (OSC 99): chunks make one event, replies go to onData", async () => {
  const t = new Terminal();
  const got: unknown[] = [];
  t.onNotification((n) => got.push(n));
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  const closes: string[] = [];
  t.onNotificationClose((id) => closes.push(id));
  const queries: string[] = [];
  t.onNotificationQuery((id) => queries.push(id));
  const alive: string[] = [];
  t.onNotificationAlive((id) => alive.push(id));

  await write(
    t,
    "\x1b]99;i=1:d=0:u=2:a=report:c=1;Hello\x1b\\\x1b]99;i=1:p=body;This is a body\x1b\\",
  );
  assert.deepEqual(got, [
    {
      id: "1",
      title: "Hello",
      body: "This is a body",
      urgency: 2,
      actions: { focus: true, report: true },
      when: "always",
      reportClose: true,
      appName: undefined,
      types: [],
      timeout: undefined,
      sound: undefined,
      iconNames: [],
      iconCacheId: undefined,
      buttons: [],
    },
  ]);

  await write(t, "\x1b]99;i=1:p=close;\x1b\\\x1b]99;i=q:p=?;\x1b\\\x1b]99;p=alive;\x1b\\");
  assert.deepEqual([closes, queries, alive], [["1"], ["q"], [""]]);

  t.notificationActivated("1");
  t.notificationActivated("1", 2);
  t.notificationClosed("1");
  t.replyNotificationQuery("q");
  t.replyNotificationAlive("", ["1", "2"]);
  assert.deepEqual(out, [
    "\x1b]99;i=1;\x1b\\",
    "\x1b]99;i=1;2\x1b\\",
    "\x1b]99;i=1:p=close;\x1b\\",
    "\x1b]99;i=q:p=?;a=focus,report:c=1:o=always,unfocused,invisible:p=title,body,?,close,alive:u=0,1,2\x1b\\",
    "\x1b]99;p=alive;1,2\x1b\\",
  ]);
  t.dispose();
});

test("notifications without a listener do nothing", async () => {
  const t = new Terminal();
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  await write(t, "\x1b]99;;hi\x1b\\\x1b]99;i=q:p=?;\x1b\\");
  assert.deepEqual(out, []);
  t.dispose();
});

const b64 = (s: string | Uint8Array) =>
  Buffer.from(typeof s === "string" ? s : s).toString("base64");

test("clipboard (OSC 5522): a write becomes one event, the answer goes to onData", async () => {
  const t = new Terminal();
  const writes: unknown[] = [];
  t.onClipboardWrite((w) => writes.push(w));
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));

  await write(
    t,
    `\x1b]5522;type=write:id=w1\x1b\\` +
      `\x1b]5522;type=wdata:mime=${b64("text/plain")};${b64("hel")}\x1b\\` +
      `\x1b]5522;type=wdata:mime=${b64("text/plain")};${b64("lo")}\x1b\\` +
      `\x1b]5522;type=wdata:mime=${b64("image/png")};${b64(new Uint8Array([137, 80, 78, 71]))}\x1b\\`,
  );
  assert.deepEqual(writes, []);
  await write(t, "\x1b]5522;type=wdata\x1b\\");
  assert.deepEqual(writes, [
    {
      id: "w1",
      location: "clipboard",
      items: [
        { mime: "text/plain", data: new TextEncoder().encode("hello") },
        { mime: "image/png", data: new Uint8Array([137, 80, 78, 71]) },
      ],
    },
  ]);
  assert.deepEqual(out, []);
  t.replyClipboardWrite("w1");
  t.replyClipboardWrite("w1", "EPERM");
  assert.deepEqual(out, [
    "\x1b]5522;type=write:status=DONE:id=w1\x1b\\",
    "\x1b]5522;type=write:status=EPERM:id=w1\x1b\\",
  ]);
  t.dispose();
});

test("clipboard (OSC 5522): primary selection and invalid base64", async () => {
  const t = new Terminal();
  const writes: { location: string }[] = [];
  t.onClipboardWrite((w) => writes.push(w));
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  await write(
    t,
    `\x1b]5522;type=write:loc=primary\x1b\\\x1b]5522;type=wdata\x1b\\` +
      `\x1b]5522;type=write\x1b\\\x1b]5522;type=wdata:mime=${b64("a/b")};!!!!\x1b\\\x1b]5522;type=wdata\x1b\\`,
  );
  assert.deepEqual(writes.map((w) => w.location), ["primary"]);
  assert.deepEqual(out, ["\x1b]5522;type=write:status=EINVAL\x1b\\"]);
  t.dispose();
});

test("clipboard (OSC 5522): read requests and answers", async () => {
  const t = new Terminal();
  const reads: unknown[] = [];
  t.onClipboardRead((r) => reads.push(r));
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  await write(
    t,
    `\x1b]5522;type=read:id=r1;${b64("text/plain image/png")}\x1b\\` +
      `\x1b]5522;type=read:loc=primary;${b64(".")}\x1b\\`,
  );
  assert.deepEqual(reads, [
    { id: "r1", location: "clipboard", list: false, mimes: ["text/plain", "image/png"] },
    { id: "", location: "primary", list: true, mimes: [] },
  ]);
  t.replyClipboardReadData("r1", [
    { mime: "text/plain", data: new TextEncoder().encode("hi") },
  ]);
  t.replyClipboardReadTypes("", ["text/plain", "image/png"]);
  t.replyClipboardReadError("r2", "EPERM");
  t.replyClipboardReadData("r3", []);
  assert.deepEqual(out, [
    `\x1b]5522;type=read:status=OK:id=r1\x1b\\` +
      `\x1b]5522;type=read:status=DATA:mime=${b64("text/plain")}:id=r1;aGk=\x1b\\` +
      `\x1b]5522;type=read:status=DONE:id=r1\x1b\\`,
    `\x1b]5522;type=read:status=OK\x1b\\` +
      `\x1b]5522;type=read:status=DATA:mime=${b64(".")};${b64("text/plain image/png")}\x1b\\` +
      `\x1b]5522;type=read:status=DONE\x1b\\`,
    "\x1b]5522;type=read:status=EPERM:id=r2\x1b\\",
    "\x1b]5522;type=read:status=OK:id=r3\x1b\\\x1b]5522;type=read:status=DONE:id=r3\x1b\\",
  ]);
  t.dispose();
});

test("clipboard (OSC 5522): without a listener, reads and writes are refused", async () => {
  const t = new Terminal();
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  await write(
    t,
    `\x1b]5522;type=read:id=r;${b64("text/plain")}\x1b\\` +
      `\x1b]5522;type=write:id=w\x1b\\\x1b]5522;type=wdata\x1b\\`,
  );
  assert.equal(
    out.join(""),
    "\x1b]5522;type=read:status=EPERM:id=r\x1b\\\x1b]5522;type=write:status=EPERM:id=w\x1b\\",
  );
  t.dispose();
});

test("clipboard (OSC 52): a write goes to onClipboardWrite with no reply", async () => {
  const t = new Terminal();
  const writes: { id: string; location: string; items: unknown }[] = [];
  t.onClipboardWrite((w) => writes.push(w));
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  await write(t, `\x1b]52;c;${b64("hello 日本")}\x07`);
  assert.equal(writes.length, 1);
  assert.equal(writes[0]!.location, "clipboard");
  assert.deepEqual(writes[0]!.items, [
    { mime: "text/plain", data: new TextEncoder().encode("hello 日本") },
  ]);
  t.replyClipboardWrite(writes[0]!.id);
  assert.deepEqual(out, []);
  t.dispose();
});

test("clipboard (OSC 52): a read goes to onClipboardRead and is answered as OSC 52", async () => {
  const t = new Terminal();
  const reads: { id: string; location: string; list: boolean; mimes: string[] }[] = [];
  t.onClipboardRead((r) => reads.push(r));
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  await write(t, "\x1b]52;c;?\x1b\\\x1b]52;c;?\x07");
  assert.deepEqual(
    reads.map(({ location, list, mimes }) => ({ location, list, mimes })),
    [
      { location: "clipboard", list: false, mimes: ["text/plain"] },
      { location: "clipboard", list: false, mimes: ["text/plain"] },
    ],
  );
  t.replyClipboardReadData(reads[0]!.id, [
    { mime: "text/plain", data: new TextEncoder().encode("hi") },
  ]);
  t.replyClipboardReadError(reads[1]!.id, "EPERM");
  assert.deepEqual(out, ["\x1b]52;c;aGk=\x1b\\", "\x1b]52;c;\x07"]);
  t.dispose();
});

test("clipboard (OSC 52): without a listener, a read gets an empty reply", async () => {
  const t = new Terminal();
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  await write(t, `\x1b]52;c;?\x07\x1b]52;c;${b64("x")}\x07`);
  assert.equal(out.join(""), "\x1b]52;c;\x07");
  t.dispose();
});

const osc72 = (meta: string, payload = "") =>
  `\x1b]72;${meta}${payload ? ";" + payload : ""}\x1b\\`;

test("drag and drop (OSC 72): support query is answered before DA1", async () => {
  const t = new Terminal();
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  await write(t, osc72("t=q:i=3") + "\x1b[c");
  assert.equal(out.join(""), "\x1b]72;t=q:i=3\x1b\\\x1b[?1;2c");
  t.dispose();
});

test("drag and drop (OSC 72): accepting a drop and reading its data", async () => {
  const t = new Terminal();
  const events: unknown[] = [];
  t.onDnd((e) => events.push(e));
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));

  assert.equal(t.dropAccepting, false);
  assert.equal(t.dropMove({ x: 1, y: 1, pixelX: 8, pixelY: 16 }, ["text/plain"]), false);
  await write(t, osc72("t=a", "text/plain"));
  assert.equal(t.dropAccepting, true);
  assert.deepEqual(t.dropAcceptedMimes, ["text/plain"]);

  const loc = { x: 2, y: 3, pixelX: 20, pixelY: 60 };
  assert.equal(t.dropMove(loc, ["text/plain", "text/uri-list"]), true);
  await write(t, osc72("t=m:o=1", "text/plain"));
  assert.equal(t.dropDrop(loc, ["text/plain", "text/uri-list"], { copy: true }), true);
  assert.deepEqual(out, [
    "\x1b]72;t=m:x=2:y=3:X=20:Y=60:o=3;text/plain text/uri-list\x1b\\",
    "\x1b]72;t=M:x=2:y=3:X=20:Y=60:o=1;text/plain text/uri-list\x1b\\",
  ]);
  out.length = 0;

  await write(t, osc72("t=r:x=1"));
  assert.deepEqual(events.slice(0, 3), [
    { type: "dropEnabled", mimes: ["text/plain"] },
    { type: "dropResponse", operation: "copy", mimes: ["text/plain"] },
    { type: "dropRequest", request: { type: "data", index: 1 } },
  ]);
  t.replyDropData(
    { type: "data", index: 1 },
    { kind: "mime" },
    new TextEncoder().encode("hi"),
  );
  assert.deepEqual(out, [
    `\x1b]72;t=r:x=1:m=1;${b64("hi")}\x1b\\\x1b]72;t=r:x=1:m=0\x1b\\`,
  ]);
  out.length = 0;
  t.replyDropError({ type: "data", index: 1 }, "EIO", "oops");
  assert.deepEqual(out, ["\x1b]72;t=R:x=1;EIO:oops\x1b\\"]);

  await write(t, osc72("t=r:o=2"));
  assert.deepEqual(events[3], { type: "dropFinished", operation: "move" });
  await write(t, osc72("t=A"));
  assert.equal(t.dropAccepting, false);
  t.dispose();
});

test("drag and drop (OSC 72): a data request with no listener is refused", async () => {
  const t = new Terminal();
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  await write(t, osc72("t=a"));
  const loc = { x: 0, y: 0, pixelX: 0, pixelY: 0 };
  t.dropMove(loc, ["text/plain"]);
  t.dropDrop(loc, ["text/plain"]);
  out.length = 0;
  await write(t, osc72("t=r:x=1"));
  assert.deepEqual(out, ["\x1b]72;t=R:x=1;EPERM\x1b\\"]);
  t.dispose();
});

test("drag and drop (OSC 72): starting a drag", async () => {
  const t = new Terminal();
  const events: any[] = [];
  t.onDnd((e) => events.push(e));
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));

  await write(t, osc72("t=o:x=1", "1:abc"));
  assert.equal(t.dragOffersEnabled, true);
  assert.equal(t.dragMachineId, "1:abc");
  assert.equal(t.dragGesture({ x: 4, y: 5, pixelX: 40, pixelY: 100 }), true);
  await write(
    t,
    osc72("t=o:o=3", "text/plain text/uri-list") +
      osc72("t=p:x=0", b64("hello")) +
      osc72("t=p:x=-1:y=32:X=1:Y=1", b64(new Uint8Array([1, 2, 3, 4]))) +
      osc72("t=P:x=-1"),
  );
  assert.deepEqual(out, ["\x1b]72;t=o:x=4:y=5:X=40:Y=100\x1b\\"]);
  out.length = 0;
  const start = events.at(-1);
  assert.equal(start.type, "dragStart");
  assert.deepEqual(start.operations, { copy: true, move: true });
  assert.deepEqual(start.mimes, ["text/plain", "text/uri-list"]);
  assert.deepEqual(start.data, [new TextEncoder().encode("hello"), null]);
  assert.deepEqual(start.images, [
    { format: "rgba", width: 1, height: 1, opacity: 0, data: new Uint8Array([1, 2, 3, 4]) },
  ]);
  assert.equal(start.image, 0);
  assert.equal(start.machineId, "1:abc");

  assert.equal(t.dragStarted(), true);
  t.dragAccepted(0);
  t.dragAction("move");
  t.dragDropped();
  t.requestDragData(1);
  assert.deepEqual(out, [
    "\x1b]72;t=E;OK\x1b\\",
    "\x1b]72;t=e:x=1:y=0\x1b\\",
    "\x1b]72;t=e:x=2:o=2\x1b\\",
    "\x1b]72;t=e:x=3\x1b\\",
    "\x1b]72;t=e:x=5:y=1\x1b\\",
  ]);
  await write(t, osc72("t=e:y=1:m=1", b64("ab")) + osc72("t=e:y=1:m=0"));
  assert.deepEqual(events.at(-1), {
    type: "dragData",
    index: 1,
    data: new TextEncoder().encode("ab"),
  });
  await write(t, osc72("t=E:y=-1"));
  assert.deepEqual(events.at(-1), { type: "dragCanceled" });
  assert.equal(t.dragFinished(), false);
  t.dispose();
});

test("drag and drop (OSC 72): a drag start with no listener is refused", async () => {
  const t = new Terminal();
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  await write(t, osc72("t=o:x=1"));
  t.dragGesture({ x: 0, y: 0, pixelX: 0, pixelY: 0 });
  out.length = 0;
  await write(t, osc72("t=o:o=1", "text/plain") + osc72("t=P:x=-1"));
  assert.deepEqual(out, ["\x1b]72;t=E;EPERM\x1b\\"]);
  assert.equal(t.dragStarted(), false);
  t.dispose();
});

test("multiple cursors: set, query and clear through the Terminal", async () => {
  const t = new Terminal({ cols: 20, rows: 10 });
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  // 仕様の例: 主のカーソルの形に従う
  await write(t, "\x1b[6 q\x1b[>29;2:4:5 q");
  assert.deepEqual(t.extraCursors, [{ x: 4, y: 3, shape: "bar" }]);
  // 仕様の例: 7 行目に、いろいろな形
  await write(t, "\x1b[>0;4 q\x1b[>1;2:7:1 q\x1b[>2;2:7:3 q\x1b[>3;2:7:5;2:7:7 q");
  assert.deepEqual(
    t.extraCursors.map((c) => [c.x, c.y, c.shape]),
    [
      [0, 6, "block"],
      [2, 6, "bar"],
      [4, 6, "underline"],
      [6, 6, "underline"],
    ],
  );
  await write(t, "\x1b[>100 q\x1b[> q");
  assert.equal(
    out.join(""),
    "\x1b[>100;1:2:7:1;2:2:7:3;3:2:7:5;3:2:7:7 q" +
      "\x1b[>1;2;3;29;30;40;100;101 q",
  );
  // 画面を消すと、全部消える
  await write(t, "\x1b[2J");
  assert.deepEqual(t.extraCursors, []);
  t.dispose();
});

const osc5113 = (body: string) => `\x1b]5113;${body}\x1b\\`;

test("file transfer (OSC 5113): a send session runs from start to finish", async () => {
  const t = new Terminal();
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  const events: FileTransferEvent[] = [];
  t.onFileTransfer((e) => events.push(e));
  await write(
    t,
    osc5113("ac=send;id=s1") +
      osc5113(`ac=file;id=s1;fid=f1;n=${b64("/tmp/a")};mod=1700000000000000000;prm=420;zip=zlib`) +
      osc5113("ac=end_data;id=s1;fid=f1;d=AQID") +
      osc5113("ac=finish;id=s1"),
  );
  assert.deepEqual(events, [
    { type: "send", id: "s1", bypass: null, quiet: 0, paths: null },
    {
      type: "file",
      id: "s1",
      fileId: "f1",
      name: "/tmp/a",
      fileType: "regular",
      compression: "zlib",
      transmission: "simple",
      mtime: 1700000000000000000n,
      permissions: 420,
      size: null,
      parent: null,
    },
    { type: "data", id: "s1", fileId: "f1", data: new Uint8Array([1, 2, 3]), end: true },
    { type: "finish", id: "s1" },
  ]);
  t.replyFileTransferOk("s1");
  t.replyFileTransferStarted("s1", "f1");
  t.replyFileTransferOk("s1", { fileId: "f1", size: 3 });
  t.replyFileTransferError("s1", "EIO", { fileId: "f1", message: "bad" });
  assert.equal(
    out.join(""),
    osc5113(`ac=status;id=s1;st=${b64("OK")}`) +
      osc5113(`ac=status;id=s1;fid=f1;st=${b64("STARTED")}`) +
      osc5113(`ac=status;id=s1;fid=f1;st=${b64("OK")};sz=3`) +
      osc5113(`ac=status;id=s1;fid=f1;st=${b64("EIO:bad")}`),
  );
  t.dispose();
});

test("file transfer (OSC 5113): a receive session sends the listing and the data", async () => {
  const t = new Terminal();
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  const events: FileTransferEvent[] = [];
  t.onFileTransfer((e) => events.push(e));
  await write(
    t,
    osc5113("ac=receive;id=r1;sz=1") + osc5113(`ac=file;id=r1;fid=f1;n=${b64("/d")}`),
  );
  assert.equal(events[0].type, "receive");
  t.replyFileTransferOk("r1");
  t.replyFileTransferListing("r1", {
    requestId: "f1",
    entryId: "e1",
    name: "/d/a",
    fileType: "regular",
    size: 5,
  });
  t.replyFileTransferOk("r1", { name: "/home/u" });
  t.replyFileTransferData("r1", "e1", new TextEncoder().encode("hello"), true);
  assert.equal(
    out.join(""),
    osc5113(`ac=status;id=r1;st=${b64("OK")}`) +
      osc5113(`ac=file;id=r1;fid=f1;st=${b64("e1")};n=${b64("/d/a")};ft=regular;sz=5`) +
      osc5113(`ac=status;id=r1;st=${b64("OK")};n=${b64("/home/u")}`) +
      osc5113(`ac=end_data;id=r1;fid=e1;d=${b64("hello")}`),
  );
  t.dispose();
});

test("file transfer (OSC 5113): cancel, and without a listener sessions are refused", async () => {
  const t = new Terminal();
  const out: string[] = [];
  t.onData((d) => out.push(dec.decode(d)));
  await write(
    t,
    osc5113("ac=send;id=a") + osc5113("ac=receive;id=b") + osc5113("ac=cancel;id=c"),
  );
  assert.equal(
    out.join(""),
    osc5113(`ac=status;id=a;st=${b64("EPERM:User refused the transfer")}`) +
      osc5113(`ac=status;id=b;st=${b64("EPERM:User refused the transfer")}`) +
      osc5113(`ac=status;id=c;st=${b64("CANCELED")}`),
  );
  t.dispose();
});
