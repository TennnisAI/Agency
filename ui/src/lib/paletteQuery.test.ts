import { describe, expect, it } from "vitest";
import { parsePaletteQuery } from "./paletteQuery";

describe("parsePaletteQuery", () => {
  it("treats plain text as the default mode", () => {
    expect(parsePaletteQuery("")).toEqual({ mode: "default", term: "" });
    expect(parsePaletteQuery("senba")).toEqual({ mode: "default", term: "senba" });
    expect(parsePaletteQuery("  padded ")).toEqual({ mode: "default", term: "padded" });
  });

  it("routes each prefix to its provider and strips it", () => {
    expect(parsePaletteQuery(">new agent")).toEqual({ mode: "command", term: "new agent" });
    expect(parsePaletteQuery("@age-14")).toEqual({ mode: "issue", term: "age-14" });
    expect(parsePaletteQuery("#journal")).toEqual({ mode: "tag", term: "journal" });
    expect(parsePaletteQuery("/resolve_within")).toEqual({ mode: "content", term: "resolve_within" });
  });

  it("trims the remainder after the prefix", () => {
    expect(parsePaletteQuery("> settings ")).toEqual({ mode: "command", term: "settings" });
  });

  it("a lone prefix is its mode with an empty term", () => {
    expect(parsePaletteQuery(">")).toEqual({ mode: "command", term: "" });
    expect(parsePaletteQuery("@")).toEqual({ mode: "issue", term: "" });
    expect(parsePaletteQuery("#")).toEqual({ mode: "tag", term: "" });
    expect(parsePaletteQuery("/")).toEqual({ mode: "content", term: "" });
  });

  it("only the first character selects a mode", () => {
    expect(parsePaletteQuery("fix > bug").mode).toBe("default");
  });
});
