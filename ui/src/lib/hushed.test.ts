import { describe, expect, it } from "vitest";
import { HUSHABLE, isHushed, setHushed } from "./hushed";

function fakeStorage() {
  const map = new Map<string, string>();
  return {
    getItem: (k: string) => map.get(k) ?? null,
    setItem: (k: string, v: string) => { map.set(k, v); },
    removeItem: (k: string) => { map.delete(k); },
    keys: () => [...map.keys()],
  };
}

describe("hushed", () => {
  it("shows every message until one is switched off", () => {
    const s = fakeStorage();
    for (const h of HUSHABLE) expect(isHushed(h.id, s)).toBe(false);
  });

  it("remembers a hushed message and forgets it again", () => {
    const s = fakeStorage();
    setHushed("merge-delete", true, s);
    expect(isHushed("merge-delete", s)).toBe(true);
    // Only the one that was silenced.
    expect(isHushed("merge-cleanup", s)).toBe(false);
    setHushed("merge-delete", false, s);
    expect(isHushed("merge-delete", s)).toBe(false);
    // Un-hushing clears the key rather than storing a "0" nobody reads.
    expect(s.keys()).toEqual([]);
  });

  it("survives storage that throws", () => {
    const broken = {
      getItem: () => { throw new Error("nope"); },
      setItem: () => { throw new Error("nope"); },
      removeItem: () => { throw new Error("nope"); },
    };
    expect(() => setHushed("merge-cleanup", true, broken)).not.toThrow();
    expect(isHushed("merge-cleanup", broken)).toBe(false);
  });
});
