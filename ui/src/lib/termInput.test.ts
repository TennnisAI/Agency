import { describe, it, expect } from "vitest";
import { createInputWriter, type Schedule } from "./termInput";

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

describe("createInputWriter", () => {
  it("sends a frame's writes as one, in order", () => {
    const sent: string[] = [];
    const clock = manualSchedule();
    const w = createInputWriter((d) => sent.push(d), clock.schedule);

    w.write("\x1b[<64;10;10M");
    w.write("\x1b[<64;10;10M");
    w.write("\x1b[<64;10;10M");
    expect(sent).toEqual([]); // nothing goes out mid-frame

    clock.run();
    expect(sent).toEqual(["\x1b[<64;10;10M\x1b[<64;10;10M\x1b[<64;10;10M"]);
  });

  it("keeps later frames separate", () => {
    const sent: string[] = [];
    const clock = manualSchedule();
    const w = createInputWriter((d) => sent.push(d), clock.schedule);

    w.write("a");
    clock.run();
    w.write("b");
    clock.run();
    expect(sent).toEqual(["a", "b"]);
  });

  it("stays quiet when nothing was written", () => {
    const sent: string[] = [];
    const clock = manualSchedule();
    createInputWriter((d) => sent.push(d), clock.schedule);
    expect(clock.pending).toBe(false);
    expect(sent).toEqual([]);
  });

  it("ignores empty writes rather than scheduling for them", () => {
    const sent: string[] = [];
    const clock = manualSchedule();
    const w = createInputWriter((d) => sent.push(d), clock.schedule);

    w.write("");
    expect(clock.pending).toBe(false);
    clock.run();
    expect(sent).toEqual([]);
  });

  it("flushes on dispose so a keystroke on the last frame still lands", () => {
    const sent: string[] = [];
    const clock = manualSchedule();
    const w = createInputWriter((d) => sent.push(d), clock.schedule);

    w.write("\r");
    w.dispose();
    expect(sent).toEqual(["\r"]);
    // and the cancelled frame does not send it a second time
    clock.run();
    expect(sent).toEqual(["\r"]);
  });

  it("sends nothing on dispose with an empty queue", () => {
    const sent: string[] = [];
    const clock = manualSchedule();
    createInputWriter((d) => sent.push(d), clock.schedule).dispose();
    expect(sent).toEqual([]);
  });
});
