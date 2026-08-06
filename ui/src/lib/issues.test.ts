import { describe, expect, it } from "vitest";
import { Issue, IssueStatus } from "../api";
import {
  NO_FILTERS,
  compareIssues,
  filtersActive,
  fmtDate,
  isOverdue,
  issueSorter,
  issuesCollapsedKey,
  issuesSelectedKey,
  loadCollapsed,
  loadSelected,
  matchRanges,
  matchesFilters,
  saveCollapsed,
  saveSelected,
  searchTerms,
  todayIssues,
} from "./issues";

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
    links: [],
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

describe("searchTerms", () => {
  it("splits on whitespace and lowercases", () => {
    expect(searchTerms("  Auth   Flake ")).toEqual(["auth", "flake"]);
    expect(searchTerms("   ")).toEqual([]);
  });
});

describe("filtersActive", () => {
  it("is false only for the untouched bar", () => {
    expect(filtersActive(NO_FILTERS)).toBe(false);
    expect(filtersActive({ ...NO_FILTERS, terms: ["x"] })).toBe(true);
    expect(filtersActive({ ...NO_FILTERS, status: "open" })).toBe(true);
    expect(filtersActive({ ...NO_FILTERS, priority: 0 })).toBe(true);
  });
});

describe("matchesFilters", () => {
  const f = (over: Partial<typeof NO_FILTERS>) => ({ ...NO_FILTERS, ...over });

  it("passes everything when nothing is set", () => {
    expect(matchesFilters(issue({ status: "cancelled" }), "AGE-1", NO_FILTERS)).toBe(true);
  });

  it("open hides done and cancelled; a status pins exactly one", () => {
    expect(matchesFilters(issue({ status: "done" }), "AGE-1", f({ status: "open" }))).toBe(false);
    expect(matchesFilters(issue({ status: "todo" }), "AGE-1", f({ status: "open" }))).toBe(true);
    expect(matchesFilters(issue({ status: "done" }), "AGE-1", f({ status: "done" }))).toBe(true);
    expect(matchesFilters(issue({ status: "todo" }), "AGE-1", f({ status: "done" }))).toBe(false);
  });

  it("matches priority exactly, -1 meaning any", () => {
    expect(matchesFilters(issue({ priority: 4 }), "AGE-1", f({ priority: 4 }))).toBe(true);
    expect(matchesFilters(issue({ priority: 3 }), "AGE-1", f({ priority: 4 }))).toBe(false);
    expect(matchesFilters(issue({ priority: 0 }), "AGE-1", f({ priority: 0 }))).toBe(true);
  });

  it("searches key, title and body, all terms required, case-insensitive", () => {
    const i = issue({ title: "Flaky login test", body: "Fails on CI only" });
    expect(matchesFilters(i, "AGE-14", f({ terms: ["flaky"] }))).toBe(true);
    expect(matchesFilters(i, "AGE-14", f({ terms: ["age-14"] }))).toBe(true);
    expect(matchesFilters(i, "AGE-14", f({ terms: ["14"] }))).toBe(true);
    expect(matchesFilters(i, "AGE-14", f({ terms: ["ci"] }))).toBe(true);
    expect(matchesFilters(i, "AGE-14", f({ terms: ["flaky", "ci"] }))).toBe(true);
    expect(matchesFilters(i, "AGE-14", f({ terms: ["flaky", "nope"] }))).toBe(false);
  });
});

describe("matchRanges", () => {
  it("finds every occurrence, case-insensitively", () => {
    expect(matchRanges("Test the test", ["test"])).toEqual([[0, 4], [9, 13]]);
  });

  it("merges overlapping terms into one span", () => {
    expect(matchRanges("logging", ["log", "ogg"])).toEqual([[0, 4]]);
  });

  it("is empty with no terms or no hit", () => {
    expect(matchRanges("anything", [])).toEqual([]);
    expect(matchRanges("anything", ["zzz"])).toEqual([]);
  });
});

describe("issueSorter", () => {
  it("board order runs status first, then the board compare", () => {
    const doing = issue({ status: "in_progress" });
    const back = issue({ status: "backlog" });
    expect([doing, back].sort(issueSorter("board")).map((i) => i.id)).toEqual([back.id, doing.id]);
  });

  it("due order puts dateless issues last", () => {
    const soon = issue({ due: "2026-08-01" });
    const later = issue({ due: "2026-09-01" });
    const none = issue({});
    expect([none, later, soon].sort(issueSorter("due")).map((i) => i.id)).toEqual([
      soon.id, later.id, none.id,
    ]);
  });

  it("updated order is newest first", () => {
    const old = issue({ updatedAt: 10 });
    const fresh = issue({ updatedAt: 99 });
    expect([old, fresh].sort(issueSorter("updated")).map((i) => i.id)).toEqual([fresh.id, old.id]);
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

describe("collapsed groups", () => {
  function makeStorage(): Pick<Storage, "getItem" | "setItem"> {
    const map = new Map<string, string>();
    return {
      getItem: (key: string) => map.get(key) ?? null,
      setItem: (key: string, value: string) => { map.set(key, value); },
    };
  }
  const KEY = issuesCollapsedKey("p1");

  it("starts with the finished groups folded", () => {
    expect([...loadCollapsed(makeStorage(), KEY)]).toEqual(["done", "cancelled"]);
  });

  it("round-trips a choice, including folding nothing", () => {
    const storage = makeStorage();
    saveCollapsed(storage, KEY, new Set<IssueStatus>(["backlog", "done"]));
    expect([...loadCollapsed(storage, KEY)]).toEqual(["backlog", "done"]);
    saveCollapsed(storage, KEY, new Set<IssueStatus>());
    expect([...loadCollapsed(storage, KEY)]).toEqual([]);
  });

  it("keeps its own key per project", () => {
    const storage = makeStorage();
    saveCollapsed(storage, KEY, new Set<IssueStatus>(["todo"]));
    expect([...loadCollapsed(storage, issuesCollapsedKey("p2"))]).toEqual(["done", "cancelled"]);
  });

  it("drops names that are no longer statuses", () => {
    const storage = makeStorage();
    storage.setItem("pane:" + KEY, "todo,archived");
    expect([...loadCollapsed(storage, KEY)]).toEqual(["todo"]);
  });
});

describe("remembered selection", () => {
  function makeStorage(): Pick<Storage, "getItem" | "setItem" | "removeItem"> {
    const map = new Map<string, string>();
    return {
      getItem: (key: string) => map.get(key) ?? null,
      setItem: (key: string, value: string) => { map.set(key, value); },
      removeItem: (key: string) => { map.delete(key); },
    };
  }
  const KEY = issuesSelectedKey("p1");

  it("has nothing open on a first visit", () => {
    expect(loadSelected(makeStorage(), KEY, [issue({})])).toBeNull();
  });

  it("round-trips the open issue", () => {
    const storage = makeStorage();
    const open = issue({});
    saveSelected(storage, KEY, open.id);
    expect(loadSelected(storage, KEY, [issue({}), open])).toBe(open.id);
  });

  it("forgets the selection once the pane is closed", () => {
    const storage = makeStorage();
    const open = issue({});
    saveSelected(storage, KEY, open.id);
    saveSelected(storage, KEY, null);
    expect(loadSelected(storage, KEY, [open])).toBeNull();
  });

  it("drops an issue that is no longer there", () => {
    const storage = makeStorage();
    saveSelected(storage, KEY, issue({}).id);
    expect(loadSelected(storage, KEY, [issue({})])).toBeNull();
  });

  it("keeps its own key per project", () => {
    const storage = makeStorage();
    const open = issue({});
    saveSelected(storage, KEY, open.id);
    expect(loadSelected(storage, issuesSelectedKey("p2"), [open])).toBeNull();
  });
});
