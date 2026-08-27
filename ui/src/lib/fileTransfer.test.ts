import { describe, expect, it } from "vitest";
import { isWithin, transferProblem } from "./fileTransfer";

describe("isWithin", () => {
  it("compares whole segments", () => {
    expect(isWithin("src", "src")).toBe(true);
    expect(isWithin("src", "src/a/b.ts")).toBe(true);
    // A name that merely starts the same is a sibling, not a child.
    expect(isWithin("src", "src2/a.ts")).toBe(false);
    expect(isWithin("src/a", "src/ab")).toBe(false);
  });

  it("treats the tree root as holding everything", () => {
    expect(isWithin("", "anything")).toBe(true);
    expect(isWithin("a", "")).toBe(false);
  });
});

describe("transferProblem", () => {
  it("refuses a folder dropped into its own subtree", () => {
    expect(transferProblem("src", "src", "move")).toBe("A folder can't go inside itself");
    expect(transferProblem("src", "src/deep", "copy")).toBe("A folder can't go inside itself");
  });

  it("refuses a move that goes nowhere, but allows the duplicate", () => {
    expect(transferProblem("src/a.ts", "src", "move")).toBe("It's already there");
    expect(transferProblem("src/a.ts", "src", "copy")).toBe(null);
    expect(transferProblem("a.ts", "", "move")).toBe("It's already there");
  });

  it("allows a real move", () => {
    expect(transferProblem("src/a.ts", "docs", "move")).toBe(null);
    expect(transferProblem("src", "src2", "move")).toBe(null);
    expect(transferProblem("src/a.ts", "", "move")).toBe(null);
  });
});
