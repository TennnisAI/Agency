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

  it("drops what is written while the snapshot is being replayed", () => {
    // The replayed snapshot re-asserts `?1004h`, and xterm answers it with a
    // focus report the moment it parses it. Nothing typed that; it must not
    // reach the child.
    const sent: string[] = [];
    const clock = manualSchedule();
    const w = createInputWriter((d) => sent.push(d), clock.schedule);

    const resume = w.suspend();
    w.write("\x1b[O");
    clock.run();
    expect(sent).toEqual([]);

    resume();
    w.write("hi");
    clock.run();
    expect(sent).toEqual(["hi"]);
  });

  it("still sends what was queued before the replay began", () => {
    const sent: string[] = [];
    const clock = manualSchedule();
    const w = createInputWriter((d) => sent.push(d), clock.schedule);

    w.write("typed");
    const resume = w.suspend();
    w.write("\x1b[O");
    clock.run();
    resume();
    expect(sent).toEqual(["typed"]);
  });

  it("resumes once however often a resume is called", () => {
    // xterm decides when a write callback runs; a resume that fired twice must
    // not unmute a replay that is still going.
    const sent: string[] = [];
    const clock = manualSchedule();
    const w = createInputWriter((d) => sent.push(d), clock.schedule);

    const first = w.suspend();
    const second = w.suspend();
    first();
    first();
    w.write("nope");
    clock.run();
    expect(sent).toEqual([]);

    second();
    w.write("yes");
    clock.run();
    expect(sent).toEqual(["yes"]);
  });
});
