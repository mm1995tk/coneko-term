import assert from "node:assert/strict";
import { test } from "node:test";
import { detectMacDesktop, inputTextareaStyle } from "../dist/input.js";

test("the hidden textarea is not transparent by opacity on macOS desktop", () => {
  const s = inputTextareaStyle(true);
  // WebKit hides the input mode indicator when the focused element has opacity < 0.01
  // or an empty visible rect
  assert.equal(s.opacity, undefined);
  assert.equal(s.width, "1px");
  assert.equal(s.height, "1px");
  assert.equal(s.color, "transparent");
  assert.equal(s.background, "transparent");
  assert.equal(s.caretColor, "transparent");
  assert.equal(s.outline, "none");
  assert.equal(s.appearance, "none");
  assert.equal(s.border, "0");
  assert.equal(s.zIndex, "-5");
});

test("the hidden textarea keeps opacity 0 elsewhere", () => {
  const s = inputTextareaStyle(false);
  assert.equal(s.opacity, "0");
  assert.equal(s.width, "0");
  assert.equal(s.height, "0");
  assert.equal(s.color, undefined);
  assert.equal(s.zIndex, "-5");
});

function withNavigator(nav: Partial<Navigator> | undefined, f: () => void): void {
  const saved = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  Object.defineProperty(globalThis, "navigator", { value: nav, configurable: true });
  try {
    f();
  } finally {
    if (saved) Object.defineProperty(globalThis, "navigator", saved);
    else delete (globalThis as { navigator?: unknown }).navigator;
  }
}

test("macOS desktop is a Mac platform without touch points", () => {
  withNavigator({ platform: "MacIntel", userAgent: "", maxTouchPoints: 0 }, () =>
    assert.equal(detectMacDesktop(), true),
  );
  // iPadOS reports MacIntel too, but has touch points
  withNavigator({ platform: "MacIntel", userAgent: "", maxTouchPoints: 5 }, () =>
    assert.equal(detectMacDesktop(), false),
  );
  withNavigator({ platform: "Linux x86_64", userAgent: "", maxTouchPoints: 0 }, () =>
    assert.equal(detectMacDesktop(), false),
  );
  withNavigator(undefined, () => assert.equal(detectMacDesktop(), false));
});
