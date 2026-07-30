import { beforeEach, describe, expect, it } from "vitest";
import { bufferKey, clearBuffers, dropBuffer, hasBuffer, stashBuffer, takeBuffer } from "./editorBuffers";

describe("editorBuffers", () => {
  beforeEach(() => clearBuffers());

  it("stash → take round-trips and clears", () => {
    stashBuffer("project:p1:src/a.ts", "edited text");
    expect(takeBuffer("project:p1:src/a.ts")).toBe("edited text");
    expect(takeBuffer("project:p1:src/a.ts")).toBeNull();
  });

  it("drop removes without returning", () => {
    stashBuffer("k", "t");
    dropBuffer("k");
    expect(takeBuffer("k")).toBeNull();
  });

  it("hasBuffer peeks without taking", () => {
    stashBuffer("k", "t");
    expect(hasBuffer("k")).toBe(true);
    expect(takeBuffer("k")).toBe("t");
    expect(hasBuffer("k")).toBe(false);
  });

  it("take on an unknown key is null", () => {
    expect(takeBuffer("nope")).toBeNull();
  });

  it("evicts the oldest entry past the cap", () => {
    for (let i = 0; i < 55; i++) stashBuffer(`k${i}`, `t${i}`);
    expect(takeBuffer("k0")).toBeNull(); // evicted
    expect(takeBuffer("k54")).toBe("t54");
  });

  it("re-stashing an existing key refreshes its age", () => {
    for (let i = 0; i < 50; i++) stashBuffer(`k${i}`, `t${i}`);
    stashBuffer("k0", "fresh"); // now newest
    stashBuffer("k99", "t99"); // evicts k1, not k0
    expect(takeBuffer("k0")).toBe("fresh");
    expect(takeBuffer("k1")).toBeNull();
  });

  it("bufferKey composes root and path", () => {
    expect(bufferKey({ kind: "project", id: "p1" }, "src/a.ts")).toBe("project:p1:src/a.ts");
  });
});
