import { describe, expect, it } from "vitest";
import {
  NEW_DRAFT, closeOutcome, createDraftStore, draftExtras, draftWaiting, issuesDraftKey, loadDraft, parseDraft,
  saveDraft,
} from "./issueDraft";

function memStorage() {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => { m.set(k, v); },
    removeItem: (k: string) => { m.delete(k); },
    m,
  };
}

describe("issue drafts", () => {
  it("round-trips through storage, per project", () => {
    const s = memStorage();
    const draft = { ...NEW_DRAFT, title: "Half a tit", priority: 3, due: "2026-10-09" };
    saveDraft(s, issuesDraftKey("p1"), draft);
    expect(loadDraft(s, issuesDraftKey("p1"))).toEqual(draft);
    expect(loadDraft(s, issuesDraftKey("p2"))).toBeNull();
  });

  it("clears the key on null rather than storing a blank draft", () => {
    const s = memStorage();
    saveDraft(s, issuesDraftKey("p1"), NEW_DRAFT);
    saveDraft(s, issuesDraftKey("p1"), null);
    expect(s.m.size).toBe(0);
  });

  it("drops what storage holds that the tracker would refuse", () => {
    expect(parseDraft("not json")).toBeNull();
    expect(parseDraft("[1]")).toBeNull();
    expect(parseDraft("null")).toBeNull();
    expect(
      parseDraft(JSON.stringify({
        title: 7, body: "kept", status: "wontfix", priority: 9, due: "next week", scheduled: "2026-1-1",
        open: "yes", extra: true,
      })),
    ).toEqual({ ...NEW_DRAFT, body: "kept", open: false });
    expect(parseDraft(JSON.stringify({ priority: 1.5 }))?.priority).toBe(0);
    expect(parseDraft(JSON.stringify({ status: "backlog", open: true }))).toEqual({
      ...NEW_DRAFT, status: "backlog",
    });
  });

  it("closes into an issue only once there is a title", () => {
    expect(closeOutcome({ ...NEW_DRAFT, title: "  Fix it " })).toBe("create");
    expect(closeOutcome({ ...NEW_DRAFT, body: "repro steps" })).toBe("keep");
    expect(closeOutcome({ ...NEW_DRAFT, title: "   ", body: "\n" })).toBe("discard");
    // Properties alone are not a draft worth keeping.
    expect(closeOutcome({ ...NEW_DRAFT, priority: 4, due: "2026-10-09" })).toBe("discard");
  });

  it("marks only a draft with something typed in it as waiting", () => {
    expect(draftWaiting(null)).toBe(false);
    expect(draftWaiting(NEW_DRAFT)).toBe(false);
    expect(draftWaiting({ ...NEW_DRAFT, body: "x", open: false })).toBe(true);
  });

  it("patches only the properties that moved off their defaults", () => {
    expect(draftExtras(NEW_DRAFT)).toBeNull();
    expect(draftExtras({ ...NEW_DRAFT, status: "backlog" })).toBeNull();
    expect(draftExtras({ ...NEW_DRAFT, priority: 2, scheduled: "2026-10-05" })).toEqual({
      priority: 2, scheduled: "2026-10-05",
    });
  });

  it("hands every reader the same draft, and tells them when it changes", () => {
    const s = memStorage();
    const draft = { ...NEW_DRAFT, title: "Stored" };
    saveDraft(s, issuesDraftKey("p1"), draft);
    const store = createDraftStore(() => s);
    const a = store.get(issuesDraftKey("p1"));
    expect(a).toEqual(draft);
    expect(store.get(issuesDraftKey("p1"))).toBe(a);
    let heard = 0;
    const off = store.subscribe(() => { heard++; });
    store.set(issuesDraftKey("p1"), null);
    expect(store.get(issuesDraftKey("p1"))).toBeNull();
    expect(s.m.size).toBe(0);
    expect(heard).toBe(1);
    off();
    store.set(issuesDraftKey("p1"), draft);
    expect(heard).toBe(1);
  });

  it("sees a save that lands while another project is showing", () => {
    const store = createDraftStore(() => memStorage());
    const a = issuesDraftKey("a");
    store.set(a, { ...NEW_DRAFT, title: "Filed" });
    store.set(issuesDraftKey("b"), { ...NEW_DRAFT, title: "Other" });
    // The save on A finishes after the board moved to B.
    store.set(a, null);
    expect(store.get(a)).toBeNull();
  });

  it("tracks a save in flight per project", () => {
    const store = createDraftStore(() => null);
    store.setSaving("a", true);
    expect(store.saving("a")).toBe(true);
    expect(store.saving("b")).toBe(false);
    store.setSaving("a", false);
    expect(store.saving("a")).toBe(false);
  });

  it("keeps drafts in memory when storage refuses access", () => {
    const store = createDraftStore(() => {
      throw new Error("SecurityError");
    });
    expect(store.get("k")).toBeNull();
    store.set("k", { ...NEW_DRAFT, title: "Kept" });
    expect(store.get("k")?.title).toBe("Kept");
  });
});
