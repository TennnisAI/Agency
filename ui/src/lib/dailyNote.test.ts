import { describe, expect, it } from "vitest";
import {
  adjacentDailyPath,
  dailyNotePath,
  dateStamp,
  defaultDailyContent,
  isDailyNotePath,
  renderDailyTemplate,
} from "./dailyNote";

const d = new Date(2026, 6, 28); // July 28, 2026 (month is 0-based)

describe("dateStamp / dailyNotePath", () => {
  it("formats local dates with zero padding", () => {
    expect(dateStamp(d)).toBe("2026-07-28");
    expect(dateStamp(new Date(2026, 0, 5))).toBe("2026-01-05");
  });

  it("builds journal paths", () => {
    expect(dailyNotePath(d)).toBe("journal/2026-07-28.md");
  });
});

describe("isDailyNotePath", () => {
  it("accepts only journal/YYYY-MM-DD.md", () => {
    expect(isDailyNotePath("journal/2026-07-28.md")).toBe(true);
    expect(isDailyNotePath("journal/notes.md")).toBe(false);
    expect(isDailyNotePath("2026-07-28.md")).toBe(false);
    expect(isDailyNotePath("other/2026-07-28.md")).toBe(false);
    expect(isDailyNotePath("journal/weekly/2026-07-28.md")).toBe(false);
  });
});

describe("adjacentDailyPath", () => {
  const paths = [
    "journal/2026-07-25.md",
    "journal/2026-07-27.md",
    "journal/2026-07-28.md",
    "journal/notes.md", // non-daily noise in the same folder
    "todo.md",
  ];

  it("finds the nearest existing note, skipping calendar gaps", () => {
    expect(adjacentDailyPath(paths, "journal/2026-07-27.md", "prev")).toBe("journal/2026-07-25.md");
    expect(adjacentDailyPath(paths, "journal/2026-07-27.md", "next")).toBe("journal/2026-07-28.md");
  });

  it("returns null at the ends and for non-daily notes", () => {
    expect(adjacentDailyPath(paths, "journal/2026-07-25.md", "prev")).toBeNull();
    expect(adjacentDailyPath(paths, "journal/2026-07-28.md", "next")).toBeNull();
    expect(adjacentDailyPath(paths, "journal/notes.md", "prev")).toBeNull();
  });

  it("navigates from a date whose own note is missing", () => {
    expect(adjacentDailyPath(paths, "journal/2026-07-26.md", "prev")).toBe("journal/2026-07-25.md");
    expect(adjacentDailyPath(paths, "journal/2026-07-26.md", "next")).toBe("journal/2026-07-27.md");
  });
});

describe("templates", () => {
  it("substitutes every {{date}}", () => {
    expect(renderDailyTemplate("# {{date}}\n\n- [ ] plan {{date}}\n", d)).toBe(
      "# 2026-07-28\n\n- [ ] plan 2026-07-28\n",
    );
  });

  it("falls back to a dated heading", () => {
    expect(defaultDailyContent(d)).toBe("# 2026-07-28\n\n");
  });
});
