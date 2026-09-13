import { describe, it, expect } from "vitest";
import { Terminal } from "@xterm/xterm";
import { paneOptions } from "./termOptions";
import { seedFrame } from "./termSeed";

const row = (term: Terminal, y: number) =>
  term.buffer.active.getLine(y)!.translateToString(true);

describe("the preview seed", () => {
  it("lands as lines in the pane's own terminal", async () => {
    // Against a real xterm, with the pane's options: the seed is written into
    // one, and `convertEol` is off there, so a bare `\n` would staircase the
    // listing off the right-hand edge one column per row.
    const term = new Terminal({ ...paneOptions(), cols: 30, rows: 5 });
    await new Promise<void>((done) => term.write(seedFrame("one\ntwo\nthree"), done));
    expect([row(term, 0), row(term, 1), row(term, 2)]).toEqual(["one", "two", "three"]);
  });

  it("drops the blank rows the daemon pads its capture with", () => {
    expect(seedFrame("one\ntwo\n\n\n")).toBe("one\r\ntwo");
    expect(seedFrame("\n\n\n")).toBe("");
    expect(seedFrame("")).toBe("");
  });

  it("does not double the carriage on a capture that already has one", () => {
    expect(seedFrame("one\r\ntwo")).toBe("one\r\ntwo");
  });
});
