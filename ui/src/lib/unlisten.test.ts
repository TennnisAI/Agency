import { describe, it, expect, vi } from "vitest";
import { unlistenQuietly } from "./unlisten";

describe("unlistenQuietly", () => {
  it("calls the unlisten it is given", () => {
    const un = vi.fn();
    unlistenQuietly(un);
    expect(un).toHaveBeenCalledOnce();
  });

  it("does nothing without one", () => {
    expect(() => unlistenQuietly(undefined)).not.toThrow();
  });

  it("takes the rejection Tauri's unregister actually produces", async () => {
    // The real shape: `UnlistenFn` is typed `() => void` and is an async
    // function, so the injected script's `listeners[eventId].handlerId` throw
    // comes back as a rejected promise. Unhandled, it reaches
    // `window.unhandledrejection` and `main.tsx` toasts it.
    const rejected = Promise.reject(new TypeError(
      "undefined is not an object (evaluating 'listeners[eventId].handlerId')",
    ));
    const un = (() => rejected) as unknown as () => void;

    // No @types/node in this project, and none is wanted for one listener.
    const proc = (globalThis as unknown as {
      process: {
        on(event: string, fn: () => void): void;
        off(event: string, fn: () => void): void;
      };
    }).process;

    const onUnhandled = vi.fn();
    proc.on("unhandledRejection", onUnhandled);
    unlistenQuietly(un);
    // A rejection is only "unhandled" once a turn of the loop has passed with
    // nothing attached, so the assertion has to outlive that.
    await new Promise((r) => setTimeout(r, 20));
    proc.off("unhandledRejection", onUnhandled);

    expect(onUnhandled).not.toHaveBeenCalled();
  });

  it("takes a synchronous throw too", () => {
    const un = () => { throw new TypeError("already gone"); };
    expect(() => unlistenQuietly(un)).not.toThrow();
  });
});
