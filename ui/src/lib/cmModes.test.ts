import { describe, expect, it } from "vitest";
import { StringStream } from "@codemirror/language";
import { simpleMode } from "@codemirror/legacy-modes/mode/simple-mode";
import type { Rule } from "@codemirror/legacy-modes/mode/simple-mode";
import { tags } from "@lezer/highlight";
import { EXTRA_LANGUAGES, TEST_MODES, TOKEN_TABLE } from "./cmModes";

/** Tokenize one line, returning [text, token] pairs with whitespace dropped. */
function tokens(states: Record<string, Rule[]>, line: string): Array<[string, string | null]> {
  const parser = simpleMode(states as unknown as { start: Rule[] });
  const state = parser.startState!(2);
  const stream = new StringStream(line, 2, 2);
  const out: Array<[string, string | null]> = [];
  let guard = 0;
  while (!stream.eol()) {
    if (++guard > 500) throw new Error("tokenizer made no progress");
    const token = parser.token(stream, state);
    const text = stream.current();
    if (text.trim()) out.push([text, token]);
    stream.start = stream.pos;
  }
  return out;
}

/** Assert that `text` was tokenized as `token` somewhere in the line. */
function has(got: Array<[string, string | null]>, text: string, token: string) {
  expect(got, JSON.stringify(got)).toContainEqual([text, token]);
}

describe("Makefile mode", () => {
  it("colors targets, assignments, variables and comments", () => {
    has(tokens(TEST_MODES.MAKE, "CFLAGS := -O2 -Wall"), "CFLAGS", "propertyName");
    has(tokens(TEST_MODES.MAKE, "build: deps # go"), "build", "typeName");
    has(tokens(TEST_MODES.MAKE, "build: deps # go"), "# go", "comment");
    has(tokens(TEST_MODES.MAKE, "\tgcc $(CFLAGS) -o $@"), "$(CFLAGS)", "constName");
    has(tokens(TEST_MODES.MAKE, ".PHONY: build"), ".PHONY", "keyword");
    has(tokens(TEST_MODES.MAKE, "ifeq ($(OS),Darwin)"), "ifeq", "controlKeyword");
  });
});

describe("Terraform mode", () => {
  it("colors block keywords, strings, numbers and attributes", () => {
    const t = tokens(TEST_MODES.HCL, 'resource "aws_s3_bucket" "b" {');
    has(t, "resource", "definitionKeyword");
    has(t, '"aws_s3_bucket"', "string");
    const u = tokens(TEST_MODES.HCL, "  count = 3 # how many");
    has(u, "count", "propertyName");
    has(u, "3", "number");
    has(u, "# how many", "comment");
    has(tokens(TEST_MODES.HCL, "  name = var.bucket_name"), "var", "constName");
    has(tokens(TEST_MODES.HCL, "  tags = merge(local.tags)"), "merge", "fnName");
  });

  it("keeps block comments open across lines", () => {
    const parser = simpleMode(TEST_MODES.HCL as unknown as { start: Rule[] });
    const state = parser.startState!(2);
    const first = new StringStream("/* opening", 2, 2);
    while (!first.eol()) { parser.token(first, state); first.start = first.pos; }
    const second = new StringStream("still comment */ count = 1", 2, 2);
    expect(parser.token(second, state)).toBe("comment");
    expect(second.current()).toBe("still comment */");
  });
});

describe("GraphQL mode", () => {
  it("colors declarations, types, fields and variables", () => {
    const t = tokens(TEST_MODES.GRAPHQL, "type Query { user(id: ID!): User }");
    has(t, "type", "keyword");
    has(t, "Query", "typeName");
    has(t, "id", "propertyName");
    has(tokens(TEST_MODES.GRAPHQL, "query Get($id: ID!) @cached {"), "$id", "variableName");
    has(tokens(TEST_MODES.GRAPHQL, "query Get($id: ID!) @cached {"), "@cached", "attributeName");
    has(tokens(TEST_MODES.GRAPHQL, "# a comment"), "# a comment", "comment");
  });
});

describe("Prisma mode", () => {
  it("colors models, scalar types and attributes", () => {
    const t = tokens(TEST_MODES.PRISMA, "model User { id String @id @default(cuid()) }");
    has(t, "model", "definitionKeyword");
    has(t, "String", "typeName");
    has(t, "@id", "attributeName");
    has(t, "@default", "attributeName");
    has(tokens(TEST_MODES.PRISMA, "// a comment"), "// a comment", "comment");
  });
});

describe("Elixir mode", () => {
  it("colors definitions, modules, atoms and attributes", () => {
    const t = tokens(TEST_MODES.ELIXIR, "defmodule My.App do");
    has(t, "defmodule", "definitionKeyword");
    has(t, "My.App", "typeName");
    has(t, "do", "controlKeyword");
    has(tokens(TEST_MODES.ELIXIR, "  @moduledoc false"), "@moduledoc", "attributeName");
    has(tokens(TEST_MODES.ELIXIR, "  {:ok, value} = fetch(url)"), ":ok", "atom");
    has(tokens(TEST_MODES.ELIXIR, "  {:ok, value} = fetch(url)"), "fetch", "fnName");
    has(tokens(TEST_MODES.ELIXIR, "  # comment"), "# comment", "comment");
  });
});

describe("Nix mode", () => {
  it("colors keywords and attribute names, and leaves // as an operator", () => {
    has(tokens(TEST_MODES.NIX, "let pkgs = import <nixpkgs> {}; in"), "let", "controlKeyword");
    has(tokens(TEST_MODES.NIX, "  description = \"a flake\";"), "description", "propertyName");
    has(tokens(TEST_MODES.NIX, "  description = \"a flake\";"), '"a flake"', "string");
    const t = tokens(TEST_MODES.NIX, "attrs // { a = 1; } # note");
    has(t, "//", "operator");
    has(t, "# note", "comment");
  });
});

// The long tail. One or two sample lines per grammar, asserting the tokens
// that prove the file's shape is understood — and, through `tokens`, that the
// tokenizer always advances (a rule that can match the empty string hangs the
// editor rather than mis-coloring it).
const TAIL_CASES: Array<[keyof typeof TEST_MODES, string, Array<[string, string]>]> = [
  ["RST", "See ``code`` and *emph* here", [["``code``", "string"], ["*emph*", "emphasis"]]],
  ["RST", ".. note:: careful", [[".. note:: careful", "definitionKeyword"]]],
  ["RST", "=======", [["=======", "heading"]]],
  ["ASCIIDOC", "== Section", [["== Section", "heading"]]],
  ["ASCIIDOC", "Use `code` and *bold* text", [["`code`", "string"], ["*bold*", "strong"]]],
  ["ASCIIDOC", ":toc: left", [[":toc: left", "propertyName"]]],
  ["TYPST", "#let x = 1 // note", [["#let", "controlKeyword"], ["1", "number"], ["// note", "comment"]]],
  ["TYPST", "= Heading", [["= Heading", "heading"]]],
  ["WIKITEXT", "== Head ==", [["== Head ==", "heading"]]],
  ["WIKITEXT", "'''bold''' and [[Link]]", [["'''bold'''", "strong"], ["[[Link", "link"]]],
  ["BIBTEX", "@article{key,", [["@article", "definitionKeyword"], ["key", "variableName"]]],
  ["BIBTEX", "title = {Hi},", [["title", "propertyName"]]],
  ["PO", 'msgid "Hello"', [["msgid", "keyword"], ['"Hello"', "string"]]],
  ["PO", "#. translator note", [["#. translator note", "comment"]]],
  ["HAML", "%div.foo{:id => 'x'}", [["%div", "tagName"], [".foo", "attributeName"], [":id", "atom"]]],
  ["CSV", 'a,"b,c",3', [["a", "variableName"], ['"b,c"', "string"], [",", "separator"], ["3", "number"]]],
  ["HJSON", "{ rate: 3, ok: true } # note", [
    ["rate", "propertyName"], ["3", "number"], ["true", "atom"], ["# note", "comment"],
  ]],
  ["CUE", "#Schema: { name: string, n: 3 }", [
    ["#Schema", "typeName"], ["name", "propertyName"], ["string", "typeName"], ["3", "number"],
  ]],
  ["JSONNET", "local x = { a:: 1 } // note", [
    ["local", "controlKeyword"], ["a", "propertyName"], ["1", "number"], ["// note", "comment"],
  ]],
  ["REG", '"Version"=dword:00000001', [
    ['"Version"', "propertyName"], ["dword", "keyword"], ["00000001", "number"],
  ]],
  ["APACHE", "<Directory /var/www>", [["<Directory", "tagName"]]],
  ["APACHE", "ServerName example.com", [["ServerName", "keyword"]]],
  ["CODEOWNERS", "*.ts @nic team@x.com", [
    ["*.ts", "string"], ["@nic", "constName"], ["team@x.com", "link"],
  ]],
  ["BAT", "REM build it", [["REM build it", "comment"]]],
  ["BAT", "if not defined X goto :end", [["if", "controlKeyword"], ["goto", "keyword"]]],
  ["BAT", "set PATH=%PATH%", [["set", "keyword"], ["%PATH%", "constName"]]],
  ["VIML", '" a comment', [['" a comment', "comment"]]],
  ["VIML", "let g:x = 'hi'", [["let", "controlKeyword"], ["g:x", "constName"], ["'hi'", "string"]]],
  ["NUSHELL", "def main [] { ls | where size > 1kb } # c", [
    ["def", "controlKeyword"], ["|", "operator"], ["1kb", "number"], ["# c", "comment"],
  ]],
  ["AWK", "/^foo/ { print $1 }", [["/^foo/", "regexp"], ["print", "controlKeyword"], ["$1", "constName"]]],
  ["AWK", 'BEGIN { FS = "," }', [["BEGIN", "definitionKeyword"], ["FS", "constName"]]],
  ["APPLESCRIPT", 'tell application "Finder" -- go', [
    ["tell", "controlKeyword"], ["application", "typeName"], ['"Finder"', "string"], ["-- go", "comment"],
  ]],
  ["KUSTO", 'Events | where Level == "Error" | summarize count()', [
    ["|", "operator"], ["where", "controlKeyword"], ['"Error"', "string"],
  ]],
  ["CODEQL", 'from Function f where f.getName() = "main" select f', [
    ["from", "controlKeyword"], ["Function", "typeName"], ['"main"', "string"], ["select", "controlKeyword"],
  ]],
  ["POLAR", "default allow = false # note", [
    ["default", "controlKeyword"], ["false", "atom"], ["# note", "comment"],
  ]],
  ["MERMAID", "graph LR", [["graph", "definitionKeyword"], ["LR", "atom"]]],
  ["MERMAID", "A[Start] --> B %% note", [["-->", "operator"], ["%% note", "comment"], ["[", "bracket"]]],
  ["LOG", "2026-08-03T14:20:00Z ERROR [main] failed after 3 retries", [
    ["2026-08-03T14:20:00Z", "number"], ["ERROR", "invalid"], ["[main]", "propertyName"],
  ]],
  ["LOG", "12:00:01 DEBUG cache warm", [["DEBUG", "comment"]]],
  ["GIT_COMMIT", "feat(ui): color the tail", [["feat(ui):", "definitionKeyword"]]],
  ["GIT_COMMIT", "# Please enter the commit message", [["# Please enter the commit message", "comment"]]],
  ["GIT_COMMIT", "Fixes: #12", [["Fixes:", "propertyName"], ["#12", "constName"]]],
  ["GIT_REBASE", "pick a1b2c3d fix the thing", [["pick", "controlKeyword"], ["a1b2c3d", "number"]]],
  ["NIM", "proc greet(name: string) =", [
    ["proc", "definitionKeyword"], ["greet", "fnName"], ["string", "typeName"],
  ]],
  ["NIM", "let x = 1 # c", [["let", "definitionKeyword"], ["1", "number"], ["# c", "comment"]]],
  ["GLEAM", "pub fn main() -> Nil {", [
    ["pub", "definitionKeyword"], ["main", "fnName"], ["->", "operator"], ["Nil", "atom"],
  ]],
  ["ADA", "procedure Main is -- go", [["procedure", "definitionKeyword"], ["-- go", "comment"]]],
  ["ADA", 'Put_Line ("Hello");', [['"Hello"', "string"]]],
  ["QML", "Rectangle { width: 100 }", [
    ["Rectangle", "typeName"], ["width", "propertyName"], ["100", "number"],
  ]],
];

describe("long-tail grammars", () => {
  it.each(TAIL_CASES)("%s colors %s", (mode, line, expected) => {
    const got = tokens(TEST_MODES[mode], line);
    for (const [text, token] of expected) has(got, text, token);
  });

  it("covers every long-tail grammar", () => {
    const covered = new Set(TAIL_CASES.map(([mode]) => mode));
    expect(Object.keys(TEST_MODES).filter((m) => !covered.has(m as keyof typeof TEST_MODES)))
      .toEqual(["MAKE", "HCL", "GRAPHQL", "PRISMA", "ELIXIR", "NIX"]);
  });
});

describe("extra language descriptions", () => {
  // A token name CodeMirror can't resolve is styled as nothing at all, and the
  // only symptom is a console warning, so check every name up front.
  it("only emits token names the highlighter knows", () => {
    // Modifiers ("function", "constant", …) are functions on `tags`, not tags,
    // so they resolve to nothing on their own; TOKEN_TABLE is where a modified
    // tag gets a name.
    const plainTags = Object.entries(tags).filter(([, tag]) => typeof tag !== "function").map(([name]) => name);
    const known = new Set([...plainTags, ...Object.keys(TOKEN_TABLE)]);
    const used = new Set<string>();
    for (const states of Object.values(TEST_MODES)) {
      for (const rules of Object.values(states)) {
        for (const rule of rules) if (typeof rule.token === "string") used.add(rule.token);
      }
    }
    expect(used.size).toBeGreaterThan(5);
    expect([...used].filter((t) => !known.has(t))).toEqual([]);
  });

  it("loads every grammar it advertises", async () => {
    for (const desc of EXTRA_LANGUAGES) {
      const support = await desc.load();
      expect(support.language, desc.name).toBeTruthy();
    }
  });
});
