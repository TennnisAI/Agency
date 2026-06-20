import { describe, expect, it } from "vitest";
import { clampWidth, loadWidth, saveWidth } from "./usePaneWidth";

// Minimal in-memory storage for testing without a DOM
function makeStorage(): Pick<Storage, "getItem" | "setItem"> {
  const map = new Map<string, string>();
  return {
    getItem: (key: string) => map.get(key) ?? null,
    setItem: (key: string, value: string) => { map.set(key, value); },
  };
}

describe("clampWidth", () => {
  it("returns value within range unchanged", () => {
    expect(clampWidth(300, 200, 400)).toBe(300);
  });
  it("clamps below min to min", () => {
    expect(clampWidth(100, 200, 400)).toBe(200);
  });
  it("clamps above max to max", () => {
    expect(clampWidth(500, 200, 400)).toBe(400);
  });
});

describe("loadWidth", () => {
  it("returns default when nothing is stored", () => {
    const storage = makeStorage();
    expect(loadWidth(storage, "sidebar", 266, 200, 460)).toBe(266);
  });

  it("returns stored value when within range", () => {
    const storage = makeStorage();
    storage.setItem("pane:sidebar", "300");
    expect(loadWidth(storage, "sidebar", 266, 200, 460)).toBe(300);
  });

  it("clamps a stored value that exceeds max", () => {
    const storage = makeStorage();
    storage.setItem("pane:sidebar", "999");
    expect(loadWidth(storage, "sidebar", 266, 200, 460)).toBe(460);
  });

  it("returns default for non-numeric stored value", () => {
    const storage = makeStorage();
    storage.setItem("pane:sidebar", "not-a-number");
    expect(loadWidth(storage, "sidebar", 266, 200, 460)).toBe(266);
  });
});

describe("saveWidth", () => {
  it("clamps an over-max value to max and persists it", () => {
    const storage = makeStorage();
    const result = saveWidth(storage, "sidebar", 999, 200, 460);
    expect(result).toBe(460);
    expect(storage.getItem("pane:sidebar")).toBe("460");
  });

  it("clamps an under-min value to min and persists it", () => {
    const storage = makeStorage();
    const result = saveWidth(storage, "sidebar", 50, 200, 460);
    expect(result).toBe(200);
    expect(storage.getItem("pane:sidebar")).toBe("200");
  });

  it("persists the clamped value and returns it", () => {
    const storage = makeStorage();
    const result = saveWidth(storage, "rail", 312, 220, 520);
    expect(result).toBe(312);
    expect(storage.getItem("pane:rail")).toBe("312");
  });
});
