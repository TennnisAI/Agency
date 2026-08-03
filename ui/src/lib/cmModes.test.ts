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

describe("extra language descriptions", () => {
  // A token name CodeMirror can't resolve is styled as nothing at all, and the
  // only symptom is a console warning, so check every name up front.
  it("only emits token names the highlighter knows", () => {
    const known = new Set([...Object.keys(tags), ...Object.keys(TOKEN_TABLE)]);
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
