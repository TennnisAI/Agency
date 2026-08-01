import { describe, expect, it } from "vitest";
import { Issue } from "../api";
import { compareIssues, fmtDate, isOverdue, todayIssues } from "./issues";

const TODAY = "2026-07-30";

let seq = 0;
function issue(over: Partial<Issue>): Issue {
  seq += 1;
  return {
    id: `id${seq}`,
    projectId: "p",
    seq,
    title: "t",
    body: "",
    status: "todo",
    priority: 0,
    due: null,
    scheduled: null,
    rank: null,
    createdAt: seq,
    updatedAt: seq,
    ...over,
  };
}

describe("compareIssues", () => {
  it("ranks first, ascending, ranked before unranked", () => {
    const a = issue({ rank: 2 });
    const b = issue({ rank: 1.5 });
    const c = issue({ priority: 4 }); // unranked, urgent
    expect([a, b, c].sort(compareIssues).map((i) => i.id)).toEqual([b.id, a.id, c.id]);
  });

  it("falls back to priority then age when unranked", () => {
    const low = issue({ priority: 1 });
    const urgent = issue({ priority: 4 });
    const older = issue({ priority: 4, createdAt: 0 });
    expect([low, urgent, older].sort(compareIssues).map((i) => i.id)).toEqual([
      older.id, urgent.id, low.id,
    ]);
  });
});

describe("isOverdue", () => {
  it("is true only for open issues past their due date", () => {
    expect(isOverdue(issue({ due: "2026-07-29" }), TODAY)).toBe(true);
    expect(isOverdue(issue({ due: TODAY }), TODAY)).toBe(false); // due today ≠ overdue
    expect(isOverdue(issue({ due: "2026-08-01" }), TODAY)).toBe(false);
    expect(isOverdue(issue({}), TODAY)).toBe(false);
    expect(isOverdue(issue({ due: "2026-07-29", status: "done" }), TODAY)).toBe(false);
  });
});

describe("fmtDate", () => {
  it("renders month-day, year only when foreign", () => {
    expect(fmtDate("2026-08-01", TODAY)).toBe("Aug 1");
    expect(fmtDate("2027-01-05", TODAY)).toBe("Jan 5 2027");
  });
  it("passes through junk from hand-edited files", () => {
    expect(fmtDate("soonish", TODAY)).toBe("soonish");
  });
});

describe("todayIssues", () => {
  it("picks due/scheduled ≤ today plus in_progress, excluding closed", () => {
    const overdue = issue({ due: "2026-07-28" });
    const dueToday = issue({ due: TODAY });
    const scheduled = issue({ scheduled: "2026-07-29" });
    const wip = issue({ status: "in_progress" });
    const future = issue({ due: "2026-08-09" });
    const doneButDue = issue({ due: "2026-07-01", status: "done" });
    const plain = issue({});
    const picked = todayIssues([plain, future, wip, scheduled, dueToday, overdue, doneButDue], TODAY);
    expect(picked.map((i) => i.id)).toEqual([overdue.id, dueToday.id, scheduled.id, wip.id]);
  });

  it("sorts earliest due first, board order among dateless", () => {
    const wipUrgent = issue({ status: "in_progress", priority: 4 });
    const wip = issue({ status: "in_progress" });
    const late = issue({ due: "2026-07-20" });
    const later = issue({ due: "2026-07-25" });
    const picked = todayIssues([wip, wipUrgent, later, late], TODAY);
    expect(picked.map((i) => i.id)).toEqual([late.id, later.id, wipUrgent.id, wip.id]);
  });
});
