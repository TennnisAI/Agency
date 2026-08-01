import { describe, expect, it } from "vitest";
import { monthGrid, monthLabel, shiftMonth } from "./calendar";

describe("monthGrid", () => {
  it("is Monday-first with a constant 6x7 shape", () => {
    // July 2026 starts on a Wednesday → the grid leads with Mon Jun 29.
    const weeks = monthGrid(2026, 7);
    expect(weeks).toHaveLength(6);
    expect(weeks.every((w) => w.length === 7)).toBe(true);
    expect(weeks[0][0].date).toBe("2026-06-29");
    expect(weeks[0][0].inMonth).toBe(false);
    expect(weeks[0][2].date).toBe("2026-07-01");
    expect(weeks[0][2].inMonth).toBe(true);
    expect(weeks.flat().filter((c) => c.inMonth)).toHaveLength(31);
  });

  it("handles leap February", () => {
    const cells = monthGrid(2024, 2).flat();
    expect(cells.filter((c) => c.inMonth)).toHaveLength(29);
    expect(cells.some((c) => c.date === "2024-02-29" && c.inMonth)).toBe(true);
  });

  it("starts exactly on Monday when the 1st is a Monday", () => {
    // June 2026 starts on a Monday — no leading spill.
    const weeks = monthGrid(2026, 6);
    expect(weeks[0][0].date).toBe("2026-06-01");
    expect(weeks[0][0].inMonth).toBe(true);
  });
});

describe("shiftMonth", () => {
  it("wraps across year boundaries both ways", () => {
    expect(shiftMonth(2026, 1, -1)).toEqual([2025, 12]);
    expect(shiftMonth(2026, 12, 1)).toEqual([2027, 1]);
    expect(shiftMonth(2026, 7, -19)).toEqual([2024, 12]);
  });
});

describe("monthLabel", () => {
  it("names the month", () => {
    expect(monthLabel(2026, 7)).toBe("July 2026");
  });
});
