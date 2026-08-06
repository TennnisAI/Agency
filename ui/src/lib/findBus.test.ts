import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { FindRank, registerFindTarget, requestFind, requestFindStep, resetFindTargets } from "./findBus";

// The bus only ever asks an element three things — is it laid out, does it
// contain the focused node — so a stand-in is enough to exercise the rules
// without pulling in a DOM implementation.
function fakeEl(opts: { visible?: boolean; focused?: boolean } = {}): HTMLElement {
  const el = {
    offsetWidth: opts.visible === false ? 0 : 100,
    offsetHeight: opts.visible === false ? 0 : 20,
    getClientRects: () => (opts.visible === false ? [] : [{}]),
    contains: (node: unknown) => opts.focused === true && node === focused,
  };
  return el as unknown as HTMLElement;
}

let focused: unknown = null;

beforeEach(() => {
  focused = { tag: "the focused node" };
  (globalThis as { document?: unknown }).document = { get activeElement() { return focused; } };
});

afterEach(() => {
  resetFindTargets();
  delete (globalThis as { document?: unknown }).document;
});

describe("requestFind", () => {
  it("does nothing when no surface is registered", () => {
    expect(requestFind("find")).toBe(false);
  });

  it("skips a surface that isn't on screen", () => {
    const open = vi.fn();
    registerFindTarget({ host: () => fakeEl({ visible: false }), open, canReplace: true, rank: FindRank.editor });
    expect(requestFind("find")).toBe(false);
    expect(open).not.toHaveBeenCalled();
  });

  it("picks the higher-ranked surface when neither has focus", () => {
    const list = vi.fn();
    const editor = vi.fn();
    registerFindTarget({ host: () => fakeEl(), open: editor, canReplace: true, rank: FindRank.editor });
    registerFindTarget({ host: () => fakeEl(), open: list, canReplace: false, rank: FindRank.list });
    requestFind("find");
    expect(list).toHaveBeenCalledOnce();
    expect(editor).not.toHaveBeenCalled();
  });

  it("lets focus override rank", () => {
    const list = vi.fn();
    const editor = vi.fn();
    registerFindTarget({ host: () => fakeEl({ focused: true }), open: editor, canReplace: true, rank: FindRank.editor });
    registerFindTarget({ host: () => fakeEl(), open: list, canReplace: false, rank: FindRank.list });
    requestFind("find");
    expect(editor).toHaveBeenCalledOnce();
    expect(list).not.toHaveBeenCalled();
  });

  it("ignores a focus-only surface that doesn't have focus", () => {
    const terminal = vi.fn();
    registerFindTarget({
      host: () => fakeEl(), open: terminal, canReplace: false, rank: FindRank.terminal, requiresFocus: true,
    });
    expect(requestFind("find")).toBe(false);
  });

  it("prefers a replace-capable surface for Find and Replace", () => {
    const list = vi.fn();
    const editor = vi.fn();
    registerFindTarget({ host: () => fakeEl(), open: editor, canReplace: true, rank: FindRank.editor });
    registerFindTarget({ host: () => fakeEl(), open: list, canReplace: false, rank: FindRank.list });
    requestFind("replace");
    expect(editor).toHaveBeenCalledWith("replace");
    expect(list).not.toHaveBeenCalled();
  });

  it("falls back to plain find when nothing on screen can replace", () => {
    const list = vi.fn();
    registerFindTarget({ host: () => fakeEl(), open: list, canReplace: false, rank: FindRank.list });
    expect(requestFind("replace")).toBe(true);
    expect(list).toHaveBeenCalledWith("find");
  });

  it("forgets a surface once it unregisters", () => {
    const open = vi.fn();
    const off = registerFindTarget({ host: () => fakeEl(), open, canReplace: true, rank: FindRank.editor });
    off();
    expect(requestFind("find")).toBe(false);
  });
});

describe("requestFindStep", () => {
  it("steps the resolved surface", () => {
    const step = vi.fn();
    registerFindTarget({ host: () => fakeEl(), open: vi.fn(), step, canReplace: true, rank: FindRank.editor });
    expect(requestFindStep(true)).toBe(true);
    expect(step).toHaveBeenCalledWith(true);
  });

  it("reports false when the resolved surface can't step", () => {
    registerFindTarget({ host: () => fakeEl(), open: vi.fn(), canReplace: true, rank: FindRank.editor });
    expect(requestFindStep(false)).toBe(false);
  });
});
