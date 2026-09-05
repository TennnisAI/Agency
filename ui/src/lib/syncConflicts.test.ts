import { describe, expect, it } from "vitest";
import { createConflictStore, markerLines } from "./syncConflicts";

const conflict = (key: string, field = "body", detail = "the later edit won") => ({
  key,
  field,
  detail,
});

describe("markerLines", () => {
  it("groups every contested field under its issue", () => {
    expect(markerLines([conflict("AGE-1", "body"), conflict("AGE-1", "title"), conflict("AGE-2")])).toEqual({
      "AGE-1": ["body: the later edit won", "title: the later edit won"],
      "AGE-2": ["body: the later edit won"],
    });
  });
});

describe("conflict reports", () => {
  it("keeps the markers and the prompt across the board's mounts", () => {
    // The regression: both were state on `IssuesView`, which is mounted only
    // on the issues tab. The prompt says the rest are "marked on their rows",
    // and a click on Agents and back threw the prompt and the rows away
    // together, so following that instruction landed on nothing.
    const store = createConflictStore();
    store.record("p1", [conflict("AGE-1")], { auto: true });

    const after = store.forProject("p1");
    expect(after.markers).toEqual({ "AGE-1": ["body: the later edit won"] });
    expect(after.prompt).toEqual([conflict("AGE-1")]);
  });

  it("leaves both alone for a pass that decided nothing", () => {
    // On the schedule the next clean pass came a couple of minutes later, and
    // wiping the markers there left nothing recording what was overwritten.
    const store = createConflictStore();
    store.record("p1", [conflict("AGE-1")], { auto: true });
    const after = store.record("p1", [], { auto: true });

    expect(after.markers).toEqual({ "AGE-1": ["body: the later edit won"] });
    expect(after.prompt).toEqual([conflict("AGE-1")]);
  });

  it("raises a prompt only for a pass nobody was watching", () => {
    // A manual pass reports its conflicts in a toast to the person who just
    // pressed Sync, and clears any prompt naming the conflicts it superseded.
    const store = createConflictStore();
    store.record("p1", [conflict("AGE-1")], { auto: true });
    const after = store.record("p1", [conflict("AGE-2")], { auto: false });

    expect(after.markers).toEqual({ "AGE-2": ["body: the later edit won"] });
    expect(after.prompt).toBeNull();
  });

  it("keeps the markers when the prompt is dismissed", () => {
    // The markers are the record the prompt points at, so answering the dialog
    // is not the same as saying the merge never happened.
    const store = createConflictStore();
    store.record("p1", [conflict("AGE-1")], { auto: true });
    const after = store.dismiss("p1");

    expect(after.markers).toEqual({ "AGE-1": ["body: the later edit won"] });
    expect(after.prompt).toBeNull();
    expect(store.forProject("p1")).toEqual(after);
  });

  it("keeps one report per project", () => {
    const store = createConflictStore();
    store.record("p1", [conflict("AGE-1")], { auto: true });

    expect(store.forProject("p2")).toEqual({ markers: {}, prompt: null });
    expect(store.forProject("p1").prompt).not.toBeNull();
  });
});
