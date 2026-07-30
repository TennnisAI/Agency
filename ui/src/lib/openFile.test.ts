import { describe, expect, it } from "vitest";
import { consumePendingOpen, requestOpenFile } from "./openFile";

describe("openFile pending slot", () => {
  it("stores the latest request until consumed", () => {
    requestOpenFile({ path: "a.ts" });
    requestOpenFile({ path: "b.ts", line: 12 });
    expect(consumePendingOpen()).toEqual({ path: "b.ts", line: 12 });
    expect(consumePendingOpen()).toBeNull();
  });
});
