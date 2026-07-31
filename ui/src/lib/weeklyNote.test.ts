import { describe, expect, it } from "vitest";
import { buildWeeklyNote, isoWeekStamp, isoWeekStart, weeklyNotePath } from "./weeklyNote";

// Months are 0-based in the Date constructor.
describe("iso weeks", () => {
  it("stamps mid-year dates", () => {
    expect(isoWeekStamp(new Date(2026, 6, 31))).toBe("2026-W31"); // Friday
    expect(isoWeekStamp(new Date(2026, 6, 27))).toBe("2026-W31"); // that Monday
    expect(isoWeekStamp(new Date(2026, 7, 2))).toBe("2026-W31"); // that Sunday
  });

  it("handles year boundaries by the Thursday rule", () => {
    // 2026-01-01 is a Thursday, so its week is W01 of 2026...
    expect(isoWeekStamp(new Date(2026, 0, 1))).toBe("2026-W01");
    // ...which starts the preceding Monday, in 2025.
    expect(isoWeekStamp(new Date(2025, 11, 29))).toBe("2026-W01");
    // 2027-01-01 is a Friday; its Thursday is 2026-12-31 → W53 of 2026.
    expect(isoWeekStamp(new Date(2027, 0, 1))).toBe("2026-W53");
  });

  it("isoWeekStart returns the Monday at local midnight", () => {
    const mon = isoWeekStart(new Date(2026, 6, 31, 15, 42));
    expect([mon.getFullYear(), mon.getMonth(), mon.getDate()]).toEqual([2026, 6, 27]);
    expect([mon.getHours(), mon.getMinutes()]).toEqual([0, 0]);
    // A Monday is its own week start.
    const same = isoWeekStart(new Date(2026, 6, 27));
    expect(same.getDate()).toBe(27);
  });

  it("weeklyNotePath lands under journal/weekly", () => {
    expect(weeklyNotePath(new Date(2026, 6, 31))).toBe("journal/weekly/2026-W31.md");
  });
});

describe("buildWeeklyNote", () => {
  it("renders all sections with wikilink syntax", () => {
    const md = buildWeeklyNote("2026-W31", {
      merges: [
        { project: "Agency", subjects: ["Phase 7: cross-domain links", "Quick-add close fix"] },
        { project: "Empty", subjects: [] },
        { project: "Site", subjects: ["New landing page"] },
      ],
      issuesClosed: [{ label: "AGE-14", title: "Fix terminal resize" }],
      runsArchived: [{ id: "run-1", title: "AGE-14 Fix terminal resize", project: "Agency" }],
    });
    expect(md).toBe(
      [
        "# Week 2026-W31",
        "",
        "## Merged",
        "",
        "### Agency",
        "",
        "- Phase 7: cross-domain links",
        "- Quick-add close fix",
        "",
        "### Site",
        "",
        "- New landing page",
        "",
        "## Issues closed",
        "",
        "- [[AGE-14]] Fix terminal resize",
        "",
        "## Agent runs archived",
        "",
        "- [[run:run-1|AGE-14 Fix terminal resize]] (Agency)",
        "",
        "## Notes",
        "",
      ].join("\n"),
    );
  });

  it("an empty week keeps the header and Notes only", () => {
    expect(buildWeeklyNote("2026-W31", { merges: [], issuesClosed: [], runsArchived: [] })).toBe(
      "# Week 2026-W31\n\n## Notes\n",
    );
  });
});
