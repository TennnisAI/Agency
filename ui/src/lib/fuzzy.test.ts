import { describe, expect, it } from "vitest";
import { fuzzyFilter, fuzzyScore } from "./fuzzy";

describe("fuzzyScore", () => {
  it("tiers: basename-prefix < basename-substring < path-substring < subsequence", () => {
    const prefix = fuzzyScore("main", "src/main.rs")!;
    const nameSub = fuzzyScore("main", "src/domain.rs")!;
    const pathSub = fuzzyScore("main", "main/lib.rs")!;
    const subseq = fuzzyScore("main", "mx/ay/in.ts")!;
    expect(prefix).toBeLessThan(nameSub);
    expect(nameSub).toBeLessThan(pathSub);
    expect(pathSub).toBeLessThan(subseq);
  });

  it("returns null when the query is not even a subsequence", () => {
    expect(fuzzyScore("zzz", "src/main.rs")).toBeNull();
  });

  it("is case-insensitive", () => {
    expect(fuzzyScore("MAIN", "src/Main.rs")).toEqual(fuzzyScore("main", "src/main.rs"));
  });

  it("breaks ties by target length", () => {
    const short = fuzzyScore("app", "app.ts")!;
    const long = fuzzyScore("app", "apple.ts")!;
    expect(short).toBeLessThan(long);
  });

  it("matches subsequences in order only", () => {
    expect(fuzzyScore("fbr", "foo/bar.rs")).not.toBeNull();
    expect(fuzzyScore("rbf", "foo/bar.rs")).toBeNull();
  });
});

describe("fuzzyFilter", () => {
  const paths = ["src/domain.rs", "main/lib.rs", "src/main.rs", "docs/readme.md"];

  it("rank-sorts matches and drops non-matches", () => {
    expect(fuzzyFilter("main", paths, (p) => p)).toEqual([
      "src/main.rs", // basename prefix
      "src/domain.rs", // basename substring
      "main/lib.rs", // path substring
    ]);
  });

  it("empty query returns the head of the list unchanged", () => {
    expect(fuzzyFilter("", paths, (p) => p, 2)).toEqual(["src/domain.rs", "main/lib.rs"]);
  });

  it("caps results at the limit", () => {
    expect(fuzzyFilter("r", paths, (p) => p, 2)).toHaveLength(2);
  });
});
