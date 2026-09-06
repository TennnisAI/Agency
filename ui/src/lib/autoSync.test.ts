import { describe, expect, it } from "vitest";
import { createAutoSchedules, shouldSayKeyMismatch } from "./autoSync";

describe("autoSchedules", () => {
  it("keeps a pause and a pass time across the board's mounts", () => {
    // The regression: `IssuesView` is only mounted on the issues tab, so a
    // click on Agents and back used to hand the schedule a clean slate. It
    // re-announced "gave up after three failures" and ran a fresh pass on
    // every return, which made flipping tabs a way to sync on demand.
    const store = createAutoSchedules();
    const first = store.forProject("p1", true);
    first.paused = true;
    first.fails = 3;
    first.pushFailed = true;
    first.lastPassAt = 1000;

    expect(store.forProject("p1", true)).toEqual({
      paused: true,
      fails: 3,
      pushFailed: true,
      keyMismatchSaid: null,
      lastPassAt: 1000,
      setting: true,
    });
  });

  it("starts over when the setting itself changes", () => {
    // Turning automatic sync off and on again is the user saying to try again,
    // and it is the only thing besides a manual sync that clears a pause.
    const store = createAutoSchedules();
    store.forProject("p1", true).paused = true;
    expect(store.forProject("p1", false).paused).toBe(false);
    expect(store.forProject("p1", true).paused).toBe(false);
  });

  it("keeps one record per project", () => {
    // A pause is about one backlog's remote, so an unreachable remote on one
    // project says nothing about another.
    const store = createAutoSchedules();
    store.forProject("p1", true).paused = true;
    expect(store.forProject("p2", true).paused).toBe(false);
    expect(store.forProject("p1", true).paused).toBe(true);
  });

  it("says a key mismatch once on the schedule and every time by hand", () => {
    // The regression: the mismatch is recomputed on every pass and the same
    // one came back every two minutes, which is the toast people learn to
    // dismiss unread. The push-failure rule a few lines below it already said
    // why that must not happen.
    const s = { keyMismatchSaid: null as string | null };
    expect(shouldSayKeyMismatch(s, "AGE", true)).toBe(true);
    expect(shouldSayKeyMismatch(s, "AGE", true)).toBe(false);
    expect(shouldSayKeyMismatch(s, "AGE", false)).toBe(true);
    // A different foreign key is different news.
    expect(shouldSayKeyMismatch(s, "XYZ", true)).toBe(true);
    // A clean pass resets it, so the same key coming back is said again.
    expect(shouldSayKeyMismatch(s, null, true)).toBe(false);
    expect(shouldSayKeyMismatch(s, "XYZ", true)).toBe(true);
  });
});
