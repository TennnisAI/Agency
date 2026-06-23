import { describe, it, expect } from "vitest";
import { joinPath } from "./filePath";

describe("joinPath", () => {
  it("joins under the root with no leading slash", () => {
    expect(joinPath("", "src")).toBe("src");
  });
  it("joins nested segments with a single slash", () => {
    expect(joinPath("src", "main.rs")).toBe("src/main.rs");
    expect(joinPath("a/b", "c")).toBe("a/b/c");
  });
});
