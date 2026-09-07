import { describe, expect, it } from "vitest";
import { missingFor } from "./useFolderMissing";

describe("missingFor", () => {
  it("answers for the folder the probe was about", () => {
    expect(missingFor({ path: "/a", missing: true }, "/a")).toBe(true);
    expect(missingFor({ path: "/a", missing: false }, "/a")).toBe(false);
  });
  it("claims nothing before the first probe lands", () => {
    expect(missingFor(null, "/a")).toBeNull();
  });
  // AGE-203: the frame right after clicking another project in the sidebar.
  // The previous project's answer is still the one in state, and rendering it
  // put the missing-folder screen up over a project that is perfectly fine.
  it("discards an answer recorded for the project just left", () => {
    expect(missingFor({ path: "/gone", missing: true }, "/b")).toBeNull();
  });
  it("has nothing to say with no project selected", () => {
    expect(missingFor({ path: "/a", missing: true }, null)).toBeNull();
  });
  // Two projects on one folder share the answer, as the sidebar sweep does.
  it("answers for a second project on the same folder", () => {
    expect(missingFor({ path: "/a", missing: true }, "/a")).toBe(true);
  });
});
