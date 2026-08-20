import { describe, it, expect } from "vitest";
import { createOutputWriter } from "./termOutput";
import { type Schedule } from "./termInput";

/** A manual scheduler: `run()` plays the pending frame. */
function manualSchedule() {
  let queued: (() => void) | null = null;
  const schedule: Schedule = (fn) => {
    queued = fn;
    return () => { queued = null; };
  };
  return {
    schedule,
    get pending() { return queued !== null; },
    run() {
      const fn = queued;
      queued = null;
      fn?.();
    },
  };
}

const bytes = (s: string) => new TextEncoder().encode(s);
const text = (b: Uint8Array) => new TextDecoder().decode(b);

describe("createOutputWriter", () => {
  it("hands a frame's chunks to the terminal as one write", () => {
    const written: string[] = [];
    const clock = manualSchedule();
    const w = createOutputWriter((b) => written.push(text(b)), clock.schedule);

    // What a macOS pty does to one Cursor repaint: the erase run lands in one
    // 1024-byte read and the redraw that has to follow it in the next.
    w.write(bytes("\x1b[2K\x1b[1A\x1b[2K\x1b[G"));
    w.write(bytes("hello"));
    expect(written).toEqual([]); // nothing reaches xterm mid-frame

    clock.run();
    expect(written).toEqual(["\x1b[2K\x1b[1A\x1b[2K\x1b[Ghello"]);
  });

  it("keeps later frames separate", () => {
    const written: string[] = [];
    const clock = manualSchedule();
    const w = createOutputWriter((b) => written.push(text(b)), clock.schedule);

    w.write(bytes("a"));
    clock.run();
    w.write(bytes("b"));
    clock.run();
    expect(written).toEqual(["a", "b"]);
  });

  it("joins chunks byte for byte, without re-encoding them", () => {
    const written: Uint8Array[] = [];
    const clock = manualSchedule();
    const w = createOutputWriter((b) => written.push(b), clock.schedule);

    // A multi-byte character split across two reads has to come out whole.
    const box = bytes("─");
    w.write(box.slice(0, 1));
    w.write(box.slice(1));
    clock.run();
    expect(written.length).toBe(1);
    expect(Array.from(written[0])).toEqual(Array.from(box));
    expect(text(written[0])).toBe("─");
  });

  it("stays quiet when nothing was written", () => {
    const written: Uint8Array[] = [];
    const clock = manualSchedule();
    createOutputWriter((b) => written.push(b), clock.schedule);
    expect(clock.pending).toBe(false);
    expect(written).toEqual([]);
  });

  it("ignores empty chunks rather than scheduling for them", () => {
    const written: Uint8Array[] = [];
    const clock = manualSchedule();
    const w = createOutputWriter((b) => written.push(b), clock.schedule);

    w.write(new Uint8Array(0));
    expect(clock.pending).toBe(false);
    clock.run();
    expect(written).toEqual([]);
  });

  it("drops the queue on dispose: the pane it would paint is going away", () => {
    const written: Uint8Array[] = [];
    const clock = manualSchedule();
    const w = createOutputWriter((b) => written.push(b), clock.schedule);

    w.write(bytes("late"));
    w.dispose();
    clock.run();
    expect(written).toEqual([]);
  });

  it("schedules again after a dispose-cancelled frame", () => {
    const written: string[] = [];
    const clock = manualSchedule();
    const w = createOutputWriter((b) => written.push(text(b)), clock.schedule);

    w.write(bytes("x"));
    w.dispose();
    w.write(bytes("y"));
    expect(clock.pending).toBe(true);
    clock.run();
    expect(written).toEqual(["y"]);
  });
});
