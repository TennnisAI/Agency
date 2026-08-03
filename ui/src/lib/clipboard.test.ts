import { describe, it, expect, vi, afterEach } from "vitest";
import { clipboardText, fullClipboardText } from "./clipboard";

const XCODE_ROWS = "/tmp/A.swift\n/tmp/A.swift:83:18 Cannot find 'Fmt' in scope";

describe("fullClipboardText", () => {
  it("takes the pasteboard over the webview's truncated first item", async () => {
    // What the paste event carries for a multi-item clipboard: row one only.
    expect(await fullClipboardText("/tmp/A.swift", async () => XCODE_ROWS)).toBe(XCODE_ROWS);
  });

  it("falls back to the paste event when the clipboard holds no text", async () => {
    expect(await fullClipboardText("hello", async () => null)).toBe("hello");
    expect(await fullClipboardText("hello", async () => "")).toBe("hello");
  });

  it("falls back when the backend read fails", async () => {
    expect(await fullClipboardText("hello", async () => { throw new Error("no"); })).toBe("hello");
  });

  it("pastes nothing when neither side has text", async () => {
    expect(await fullClipboardText("", async () => null)).toBe("");
  });
});

describe("clipboardText", () => {
  const stubNavigator = (readText: () => Promise<string>) => {
    vi.stubGlobal("navigator", { clipboard: { readText } });
  };
  afterEach(() => vi.unstubAllGlobals());

  it("reads the pasteboard rather than the webview", async () => {
    stubNavigator(async () => "/tmp/A.swift");
    expect(await clipboardText(async () => XCODE_ROWS)).toBe(XCODE_ROWS);
  });

  it("asks the webview when the pasteboard read is empty or fails", async () => {
    stubNavigator(async () => "from webview");
    expect(await clipboardText(async () => null)).toBe("from webview");
    expect(await clipboardText(async () => { throw new Error("no"); })).toBe("from webview");
  });

  it("surfaces a refused webview read, so the caller can say so", async () => {
    stubNavigator(async () => { throw new Error("denied"); });
    await expect(clipboardText(async () => null)).rejects.toThrow("denied");
  });
});
