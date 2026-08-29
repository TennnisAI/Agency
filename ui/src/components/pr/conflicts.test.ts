import { describe, expect, it } from "vitest";
import { PrConflicts } from "../../api";
import { CONFLICT_LIST_CAP, conflictNote } from "./conflicts";

const probe = (files: string[], probed = true): PrConflicts => ({
  base: "main",
  head: "feat/x",
  files,
  probed,
});

describe("conflictNote", () => {
  it("waits on the probe rather than claiming a clean merge", () => {
    expect(conflictNote(null)).toBe("Checking which files collide…");
  });

  it("counts one file in the singular", () => {
    expect(conflictNote(probe(["src/a.rs"]))).toBe("One file collides: src/a.rs");
  });

  it("lists a handful in full", () => {
    expect(conflictNote(probe(["a.rs", "b.tsx", "c.css"]))).toBe(
      "3 files collide: a.rs, b.tsx, c.css",
    );
  });

  it("caps a long list and says how many are left", () => {
    const files = Array.from({ length: CONFLICT_LIST_CAP + 2 }, (_, i) => `f${i}.rs`);
    const note = conflictNote(probe(files));
    expect(note).toContain(`${files.length} files collide`);
    expect(note).toContain("and 2 more");
    expect(note).not.toContain(`f${CONFLICT_LIST_CAP}.rs`);
  });

  // The two empty-list cases mean opposite things, and reading either as "no
  // conflicts" would contradict the banner they sit inside.
  it("distinguishes a stale GitHub verdict from a probe that couldn't run", () => {
    expect(conflictNote(probe([]))).toContain("no longer shows up locally");
    expect(conflictNote(probe([], false))).toContain("couldn't work out which files collide");
  });
});
