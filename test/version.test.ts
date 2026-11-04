import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { init, version } from "../dist/index.js";

test("version returns the package version", async () => {
  // npm パッケージに入る .wasm をそのまま読む
  const wasm = await readFile(
    new URL("../dist/wasm/coneko_term_wasm_bg.wasm", import.meta.url),
  );
  await init(wasm);
  const pkg = JSON.parse(
    await readFile(new URL("../package.json", import.meta.url), "utf8"),
  );
  assert.equal(version(), pkg.version);
});
