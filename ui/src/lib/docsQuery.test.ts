import { describe, expect, it } from "vitest";
import { parseDocsQuery } from "./docsQuery";

describe("parseDocsQuery", () => {
  it("passes plain text through", () => {
    expect(parseDocsQuery("find me")).toEqual({ filters: [], text: "find me" });
  });

  it("extracts a single filter", () => {
    expect(parseDocsQuery("status:draft")).toEqual({ filters: [["status", "draft"]], text: "" });
  });

  it("extracts filters mixed with text", () => {
    expect(parseDocsQuery("status:draft budget notes type:plan")).toEqual({
      filters: [["status", "draft"], ["type", "plan"]],
      text: "budget notes",
    });
  });

  it("supports quoted values", () => {
    expect(parseDocsQuery('status:"in review" foo')).toEqual({
      filters: [["status", "in review"]],
      text: "foo",
    });
  });

  it("keeps a trailing bare key: as a has-key filter", () => {
    expect(parseDocsQuery("due:")).toEqual({ filters: [["due", ""]], text: "" });
    expect(parseDocsQuery("budget due: ")).toEqual({ filters: [["due", ""]], text: "budget" });
  });

  it("leaves prose colons as text (searching literal 'error: timeout' works)", () => {
    expect(parseDocsQuery("error: timeout")).toEqual({ filters: [], text: "error: timeout" });
    expect(parseDocsQuery("error: timeout status:draft")).toEqual({
      filters: [["status", "draft"]],
      text: "error: timeout",
    });
  });

  it("leaves URLs as text", () => {
    expect(parseDocsQuery("http://example.com")).toEqual({ filters: [], text: "http://example.com" });
    expect(parseDocsQuery("see https://x.dev docs")).toEqual({ filters: [], text: "see https://x.dev docs" });
  });

  it("leaves #tags in the text", () => {
    expect(parseDocsQuery("#alpha status:x")).toEqual({ filters: [["status", "x"]], text: "#alpha" });
  });

  it("treats a leading-digit token with a colon as text", () => {
    expect(parseDocsQuery("14:30 meeting")).toEqual({ filters: [], text: "14:30 meeting" });
  });
});
