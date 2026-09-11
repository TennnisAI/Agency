import { describe, expect, it } from "vitest";
import { detectPlatform, hasTauri, shortcutLabel } from "./platform";

describe("detectPlatform", () => {
  it("reads the platform string first and the user agent second", () => {
    expect(detectPlatform("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)", "MacIntel")).toBe("mac");
    expect(detectPlatform("Mozilla/5.0 (X11; Linux x86_64)", "Linux x86_64")).toBe("linux");
    expect(detectPlatform("Mozilla/5.0 (Windows NT 10.0; Win64; x64)", "Win32")).toBe("windows");
    // An empty platform string (some WebKit builds) falls back to the UA.
    expect(detectPlatform("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)", "")).toBe("mac");
    expect(detectPlatform("Mozilla/5.0 (X11; Linux x86_64)", "")).toBe("linux");
  });
});

describe("shortcutLabel", () => {
  it("leaves a Mac chord alone on a Mac", () => {
    expect(shortcutLabel("⌘⇧D", "mac")).toBe("⌘⇧D");
    expect(shortcutLabel("⌘↵", "mac")).toBe("⌘↵");
  });

  it("spells the same chord out for Linux and Windows", () => {
    expect(shortcutLabel("⌘K", "linux")).toBe("Ctrl+K");
    expect(shortcutLabel("⌘⇧D", "linux")).toBe("Ctrl+Shift+D");
    expect(shortcutLabel("⌥⌘F", "windows")).toBe("Ctrl+Alt+F");
    expect(shortcutLabel("⌘↵", "linux")).toBe("Ctrl+Enter");
    expect(shortcutLabel("⌘,", "linux")).toBe("Ctrl+,");
  });

  it("keeps a bare key as it is", () => {
    expect(shortcutLabel("↵", "linux")).toBe("Enter");
    expect(shortcutLabel("F11", "linux")).toBe("F11");
  });
});

describe("hasTauri", () => {
  it("is true only when the Tauri internals are on the window", () => {
    expect(hasTauri({ __TAURI_INTERNALS__: {} })).toBe(true);
    expect(hasTauri({})).toBe(false);
    expect(hasTauri(undefined)).toBe(false);
  });
});
