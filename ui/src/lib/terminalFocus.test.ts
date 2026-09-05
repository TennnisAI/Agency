import { describe, it, expect } from "vitest";
import { focusReport } from "./terminalFocus";

describe("focusReport", () => {
  it("says nothing to a child that never asked for focus reports", () => {
    expect(focusReport(false, true)).toBeNull();
    expect(focusReport(false, false)).toBeNull();
  });

  it("tells a focused pane's child that focus is here", () => {
    // The one AGE-171 turned on: without it, cursor-agent keeps showing the
    // cursor-less input line it drew when it was told the terminal blurred.
    expect(focusReport(true, true)).toBe("\x1b[I");
  });

  it("tells an unfocused pane's child that it is not", () => {
    expect(focusReport(true, false)).toBe("\x1b[O");
  });
});
