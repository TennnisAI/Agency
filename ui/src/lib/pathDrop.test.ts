import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  createSinkTracker, pathSinkAt, pathsToInput, quoteArg, registerPathSink, resetPathSinks,
} from "./pathDrop";

// The registry asks an element one thing — does it contain the node painted at
// the drop point — so a stand-in is enough, as in findBus.test.
function fakeEl(owns: unknown[]): HTMLElement {
  return { contains: (node: unknown) => owns.includes(node) } as unknown as HTMLElement;
}

const paintedAt = new Map<string, unknown>();

beforeEach(() => {
  paintedAt.clear();
  (globalThis as { document?: unknown }).document = {
    elementFromPoint: (x: number, y: number) => paintedAt.get(`${x},${y}`) ?? null,
  };
});

afterEach(() => {
  resetPathSinks();
  delete (globalThis as { document?: unknown }).document;
});

describe("pathSinkAt", () => {
  it("finds nothing when no sink is registered", () => {
    const node = {};
    paintedAt.set("10,10", node);
    expect(pathSinkAt(10, 10)).toBeNull();
  });

  it("picks the sink whose host owns what is painted there", () => {
    const inTerminal = {};
    const elsewhere = {};
    paintedAt.set("10,10", inTerminal);
    paintedAt.set("90,90", elsewhere);
    registerPathSink({ host: () => fakeEl([inTerminal]), label: "the agent", accept: vi.fn() });
    expect(pathSinkAt(10, 10)?.label).toBe("the agent");
    expect(pathSinkAt(90, 90)).toBeNull();
  });

  it("ignores a sink that has unmounted", () => {
    const node = {};
    paintedAt.set("10,10", node);
    const off = registerPathSink({ host: () => fakeEl([node]), label: "the agent", accept: vi.fn() });
    off();
    expect(pathSinkAt(10, 10)).toBeNull();
  });

  // A hidden pane (an inactive tab's terminal) still has an element, but
  // nothing of it is painted anywhere, so no point can be inside it.
  it("ignores a sink whose host is not what the point hits", () => {
    const hidden = {};
    const visible = {};
    paintedAt.set("10,10", visible);
    registerPathSink({ host: () => fakeEl([hidden]), label: "the agent", accept: vi.fn() });
    expect(pathSinkAt(10, 10)).toBeNull();
  });

  it("clears hover feedback when a sink unmounts mid-drag", () => {
    const setOver = vi.fn();
    const off = registerPathSink({ host: () => null, label: "the agent", accept: vi.fn(), setOver });
    off();
    expect(setOver).toHaveBeenCalledWith(false);
  });
});

describe("createSinkTracker", () => {
  it("lights one sink at a time as the pointer crosses them", () => {
    const a = {}, b = {};
    paintedAt.set("10,10", a);
    paintedAt.set("90,90", b);
    const overA = vi.fn(), overB = vi.fn();
    registerPathSink({ host: () => fakeEl([a]), label: "the agent", accept: vi.fn(), setOver: overA });
    registerPathSink({ host: () => fakeEl([b]), label: "the terminal", accept: vi.fn(), setOver: overB });
    const track = createSinkTracker();

    expect(track.over(10, 10)?.label).toBe("the agent");
    expect(overA).toHaveBeenLastCalledWith(true);
    track.over(90, 90);
    expect(overA).toHaveBeenLastCalledWith(false);
    expect(overB).toHaveBeenLastCalledWith(true);
    track.clear();
    expect(overB).toHaveBeenLastCalledWith(false);
  });

  it("does not re-light the sink it is already over", () => {
    const a = {};
    paintedAt.set("10,10", a);
    paintedAt.set("11,11", a);
    const overA = vi.fn();
    registerPathSink({ host: () => fakeEl([a]), label: "the agent", accept: vi.fn(), setOver: overA });
    const track = createSinkTracker();
    track.over(10, 10);
    track.over(11, 11);
    expect(overA).toHaveBeenCalledTimes(1);
  });
});

describe("quoteArg", () => {
  it("leaves an ordinary path bare", () => {
    expect(quoteArg("/Users/x/notes.md")).toBe("/Users/x/notes.md");
  });

  it("quotes a path with whitespace", () => {
    expect(quoteArg("/Users/x/My Notes/a b.md")).toBe("'/Users/x/My Notes/a b.md'");
  });

  it("closes, escapes and reopens a single quote", () => {
    expect(quoteArg("/Users/x/it's here.md")).toBe(`'/Users/x/it'\\''s here.md'`);
  });
});

describe("pathsToInput", () => {
  it("joins with spaces and leaves the cursor clear of the last path", () => {
    expect(pathsToInput(["/a/b.md", "/c/d e.md"])).toBe("/a/b.md '/c/d e.md' ");
  });
});
