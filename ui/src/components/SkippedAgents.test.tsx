import { describe, it, expect } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import SkippedAgents from "./SkippedAgents";

describe("SkippedAgents", () => {
  it("names each file and why", () => {
    const html = renderToStaticMarkup(
      <SkippedAgents skipped={[{ file: ".agency/agents/broken.md", reason: "missing frontmatter fence" }]} />,
    );
    expect(html).toContain(".agency/agents/broken.md");
    expect(html).toContain("missing frontmatter fence");
  });

  it("says nothing when every file loaded", () => {
    expect(renderToStaticMarkup(<SkippedAgents skipped={[]} />)).toBe("");
  });
});
