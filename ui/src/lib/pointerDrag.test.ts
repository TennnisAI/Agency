import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { startPointerDrag } from "./pointerDrag";

// A window that records its listeners, so the test can fire events at them
// directly: the suite runs in node, with no DOM.
function fakeWindow() {
  const listeners = new Map<string, Set<(ev: unknown) => void>>();
  return {
    listeners,
    addEventListener: (type: string, fn: (ev: unknown) => void) => {
      if (!listeners.has(type)) listeners.set(type, new Set());
      listeners.get(type)!.add(fn);
    },
    removeEventListener: (type: string, fn: (ev: unknown) => void) => {
      listeners.get(type)?.delete(fn);
    },
    getSelection: () => null,
    setTimeout: (fn: () => void) => fn,
    fire(type: string, ev: object) {
      for (const fn of [...(listeners.get(type) ?? [])]) fn(ev);
    },
  };
}

const press = { clientX: 0, clientY: 0, preventDefault: () => {} } as unknown as React.MouseEvent;
const mouse = (x: number) => ({ clientX: x, clientY: 0, preventDefault: vi.fn() });
const key = (k: string) => ({ key: k, preventDefault: vi.fn(), stopPropagation: vi.fn() });

describe("startPointerDrag", () => {
  let win: ReturnType<typeof fakeWindow>;
  beforeEach(() => {
    win = fakeWindow();
    vi.stubGlobal("window", win);
  });
  afterEach(() => vi.unstubAllGlobals());

  const handlers = () => ({ onStart: vi.fn(), onMove: vi.fn(), onCancel: vi.fn(), onDrop: vi.fn() });

  it("is a click until the pointer has moved 5px", () => {
    const h = handlers();
    startPointerDrag(press, h);
    win.fire("mousemove", mouse(4));
    expect(h.onStart).not.toHaveBeenCalled();
    win.fire("mousemove", mouse(5));
    expect(h.onStart).toHaveBeenCalledOnce();
    expect(h.onMove).toHaveBeenCalledOnce();
    win.fire("mouseup", mouse(5));
    expect(h.onDrop).toHaveBeenCalledOnce();
  });

  // The Escape that cancelled a drag also reached the focused terminal and
  // interrupted the agent mid-turn: the listener saw it and let it go on.
  it("keeps the Escape that cancels a drag to itself", () => {
    const h = handlers();
    startPointerDrag(press, h);
    win.fire("mousemove", mouse(10));
    const esc = key("Escape");
    win.fire("keydown", esc);
    expect(esc.stopPropagation).toHaveBeenCalled();
    expect(esc.preventDefault).toHaveBeenCalled();
    expect(h.onCancel).toHaveBeenCalledOnce();

    win.fire("mousemove", mouse(20));
    expect(h.onMove).toHaveBeenCalledOnce();
    win.fire("mouseup", mouse(20));
    expect(h.onDrop).not.toHaveBeenCalled();
  });

  it("lets every other key through, and every key once released", () => {
    startPointerDrag(press, handlers());
    const other = key("a");
    win.fire("keydown", other);
    expect(other.stopPropagation).not.toHaveBeenCalled();

    win.fire("mouseup", mouse(0));
    expect(win.listeners.get("keydown")?.size ?? 0).toBe(0);
  });

  it("swallows the click a drag's release fires, and not a plain click's", () => {
    startPointerDrag(press, handlers());
    win.fire("mouseup", mouse(0));
    expect(win.listeners.get("click")?.size ?? 0).toBe(0);

    startPointerDrag(press, handlers());
    win.fire("mousemove", mouse(10));
    win.fire("mouseup", mouse(10));
    const click = { stopPropagation: vi.fn(), preventDefault: vi.fn() };
    win.fire("click", click);
    expect(click.stopPropagation).toHaveBeenCalled();
  });
});
