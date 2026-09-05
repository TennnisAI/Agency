import { describe, expect, it } from "vitest";
import {
  GROUPS,
  GROUP_BY_ID,
  GROUP_IDS,
  SECTIONS,
  SECTION_IDS,
  matchSections,
  scopeOf,
} from "./settingsSections";

describe("the section index", () => {
  it("gives every section a group that exists", () => {
    for (const s of SECTIONS) expect(GROUP_BY_ID[s.group]).toBeTruthy();
  });

  // SECTION_BY_ID and GROUP_BY_ID are `as` casts over Object.fromEntries, so an
  // id in the union that the table has no row for type-checks and then throws
  // in scopeOf while rendering that section. These two are the only thing that
  // catches it.
  it("has a row for every section id", () => {
    expect(SECTIONS.map((s) => s.id).sort()).toEqual([...SECTION_IDS].sort());
  });

  it("has a row for every group id", () => {
    expect(GROUPS.map((g) => g.id).sort()).toEqual([...GROUP_IDS].sort());
  });

  it("has no duplicate ids", () => {
    expect(new Set(SECTIONS.map((s) => s.id)).size).toBe(SECTIONS.length);
    expect(new Set(GROUPS.map((g) => g.id)).size).toBe(GROUPS.length);
  });

  it("leaves no group empty, so the rail never draws a heading over nothing", () => {
    for (const g of GROUPS) expect(SECTIONS.some((s) => s.group === g.id)).toBe(true);
  });

  // The point of the whole grouping: these three write .agency/agency.local.toml
  // in one checkout, and everything else is a preference for the app.
  it("scopes exactly the per-project sections to a project", () => {
    const project = SECTIONS.filter((s) => scopeOf(s.id) === "project").map((s) => s.id);
    expect(project.sort()).toEqual(["backlog", "files", "knowledge"]);
  });
});

describe("matchSections", () => {
  it("returns null for an empty or blank query", () => {
    expect(matchSections("")).toBeNull();
    expect(matchSections("   ")).toBeNull();
  });

  it("returns an empty set when nothing matches", () => {
    expect(matchSections("kubernetes")).toEqual(new Set());
  });

  it("matches a label, case and spacing insensitively", () => {
    expect(matchSections("  APPEARANCE ")).toEqual(new Set(["appearance"]));
  });

  it("matches on terms the label never mentions", () => {
    expect(matchSections("word wrap")).toEqual(new Set(["editor"]));
    expect(matchSections("dark")).toEqual(new Set(["appearance"]));
  });

  it("matches a group label, so 'project' lists the project-scoped sections", () => {
    expect(matchSections("project")).toEqual(new Set(["backlog", "knowledge", "files"]));
  });

  it("requires every word, so a second word narrows rather than widens", () => {
    const one = matchSections("model")!;
    expect(one.size).toBeGreaterThan(1);
    expect(matchSections("model context protocol")).toEqual(new Set(["mcp"]));
  });

  it("finds a section by a name only the running app knows", () => {
    expect(matchSections("linear")).toEqual(new Set());
    expect(matchSections("linear", { mcp: "linear github" })).toEqual(new Set(["mcp"]));
  });

  it("ignores extra terms attached to a section that does not exist in the query", () => {
    expect(matchSections("agency", { backlog: "agency", knowledge: "agency", files: "agency" }))
      .toEqual(new Set(["backlog", "knowledge", "files"]));
  });

  // Both of these found nothing at all before. The all-words rule is right; it
  // is exact substring matching on a hand-written term list that made it
  // brittle, in the two directions people actually type.
  it("matches a plural of an indexed word", () => {
    expect(matchSections("themes")).toEqual(matchSections("theme"));
    expect(matchSections("colors")).toEqual(new Set(["appearance"]));
  });

  it("keeps a short word whole, where a trailing s is part of it", () => {
    // "sse" is a transport, not a plural of "ss".
    expect(matchSections("sse")).toEqual(new Set(["mcp"]));
  });

  it("drops connectors, which every section would otherwise have to contain", () => {
    expect(matchSections("check for updates")).toEqual(new Set(["diagnostics"]));
    expect(matchSections("word wrap in the file viewer")).toEqual(new Set(["editor"]));
  });

  it("searches a query that is nothing but connectors, rather than blanking", () => {
    // "the" is a prefix of "theme"; the point is that it is a search at all,
    // not the empty-box state that shows the selected section instead.
    expect(matchSections("the")).not.toBeNull();
  });

  it("finds the sections behind the phrasings that missed", () => {
    expect(matchSections("dark mode")).toEqual(new Set(["appearance"]));
    expect(matchSections("hide messages")).toEqual(new Set(["messages"]));
    expect(matchSections("stop showing")).toEqual(new Set(["messages"]));
  });

  // The imperative phrasings. "on" was already a connector and "off" was not,
  // so "turn on notifications" worked and "turn off notifications" returned
  // nothing at all.
  it("drops the verbs people put in front of a settings search", () => {
    expect(matchSections("turn off notifications")).toEqual(new Set(["notifications"]));
    expect(matchSections("where do I change the theme")).toEqual(new Set(["appearance"]));
    expect(matchSections("disable idle alerts")).toEqual(new Set(["notifications"]));
  });

  it("keeps the verbs that name something, rather than dropping them too", () => {
    // "show", "hide" and "again" are indexed on Messages; a stop word list that
    // swallowed them would answer these with every section instead of one.
    expect(matchSections("stop showing again")).toEqual(new Set(["messages"]));
    expect(matchSections("don't ask again")).toEqual(new Set(["messages"]));
    expect(matchSections("hidden messages")).toEqual(new Set(["messages"]));
  });

  it("answers with the closest sections rather than nothing", () => {
    // No term list holds every phrasing, and every gap in it used to be a blank
    // screen. "banner" is indexed, "popup" is not; the answer is still the
    // section the person was looking at.
    expect(matchSections("notification popup")).toEqual(new Set(["notifications"]));
    expect(matchSections("word wrap gibberish")).toEqual(new Set(["editor"]));
  });

  it("still returns empty when not one word hits anything", () => {
    expect(matchSections("kubernetes helm chart")).toEqual(new Set());
  });

  it("finds an agent's CLI version, which only the running app can name", () => {
    expect(matchSections("claude version")).toEqual(new Set(["diagnostics"]));
    expect(matchSections("claude version", { diagnostics: "Claude Code Codex" }))
      .toEqual(new Set(["diagnostics"]));
  });
});
