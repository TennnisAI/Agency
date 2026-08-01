import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { consumePendingOpen, fileRootKey, onOpenFile, requestOpenFile } from "./openFile";

describe("openFile pending slot", () => {
  // The node test environment has no window; an EventTarget is all the
  // module needs (add/removeEventListener + dispatchEvent).
  beforeEach(() => vi.stubGlobal("window", new EventTarget()));
  afterEach(() => vi.unstubAllGlobals());

  it("stores the latest request until consumed", () => {
    requestOpenFile({ rootKey: "project:p1", path: "a.ts" });
    requestOpenFile({ rootKey: "project:p1", path: "b.ts", line: 12 });
    expect(consumePendingOpen("project:p1")).toEqual({ rootKey: "project:p1", path: "b.ts", line: 12 });
    expect(consumePendingOpen("project:p1")).toBeNull();
  });

  it("drops a pending request aimed at another root instead of delivering it", () => {
    // A request whose view never mounted (e.g. the workspace has no Files tab)
    // must not fire later against a different project's root.
    requestOpenFile({ rootKey: "project:workspace", path: "journal/2026-08-01.md" });
    expect(consumePendingOpen("project:other")).toBeNull();
    // ...and the mismatch clears the slot for good.
    expect(consumePendingOpen("project:workspace")).toBeNull();
  });

  it("delivers live only to the matching root, leaving others pending", () => {
    const gotA: string[] = [];
    const gotB: string[] = [];
    const offA = onOpenFile("project:a", (r) => gotA.push(r.path));
    const offB = onOpenFile("project:b", (r) => gotB.push(r.path));
    requestOpenFile({ rootKey: "project:b", path: "x.ts" });
    expect(gotA).toEqual([]);
    expect(gotB).toEqual(["x.ts"]);
    // Delivered — nothing left for a later mount.
    expect(consumePendingOpen("project:b")).toBeNull();
    offA();
    offB();
  });

  it("keeps a mismatched live request pending for the right mount", () => {
    // FilesView is still showing the old root when the palette switches
    // projects and fires; the new root's mount consumes it a render later.
    const got: string[] = [];
    const off = onOpenFile("project:old", (r) => got.push(r.path));
    requestOpenFile({ rootKey: "project:new", path: "y.ts", line: 3 });
    expect(got).toEqual([]);
    off();
    expect(consumePendingOpen("project:new")).toEqual({ rootKey: "project:new", path: "y.ts", line: 3 });
  });

  it("builds root keys in the kind:id form views compare against", () => {
    expect(fileRootKey({ kind: "project", id: "p1" })).toBe("project:p1");
    expect(fileRootKey({ kind: "run", id: "r9" })).toBe("run:r9");
  });
});
