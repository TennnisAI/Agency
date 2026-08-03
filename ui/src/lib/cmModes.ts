// Grammars for file types CodeMirror's language-data doesn't ship but that
// turn up constantly in real repos (Makefiles, Terraform, GraphQL, Zig, …).
// Each is a small stream tokenizer: comments, strings, numbers and keywords,
// which is all the file editor needs to stop rendering these as flat text.
//
// Token names are @lezer/highlight tag names, so they land on the same theme
// colors as every other language (see lib/cmTheme.ts).

import { LanguageDescription, LanguageSupport, StreamLanguage, type StreamParser } from "@codemirror/language";
import { tags } from "@lezer/highlight";
import type { Rule } from "@codemirror/legacy-modes/mode/simple-mode";

type Modes = Record<string, Rule[]>;

// simple-mode rewrites a dotted token name ("variableName.function") into a
// space-separated one, which CodeMirror then reads as two base tags and drops
// the modifier. Naming the modified tags here keeps them intact.
export const TOKEN_TABLE = {
  fnName: tags.function(tags.variableName),
  constName: tags.constant(tags.variableName),
};

/** Load `simple-mode` on demand and wrap the rules as a language. */
async function simple(states: Modes): Promise<LanguageSupport> {
  const { simpleMode } = await import("@codemirror/legacy-modes/mode/simple-mode");
  // simpleMode's typing wants a literal state map; ours is built dynamically.
  return stream({ ...simpleMode(states as unknown as { start: Rule[] }), tokenTable: TOKEN_TABLE });
}

function stream(parser: StreamParser<unknown>): LanguageSupport {
  return new LanguageSupport(StreamLanguage.define(parser));
}

/** `{ word: true }` set, the shape legacy clike modes want. */
function words(list: string): Record<string, boolean> {
  const set: Record<string, boolean> = {};
  for (const w of list.split(" ")) set[w] = true;
  return set;
}

// A /* … */ comment state, shared by the C-ish modes below.
const blockComment: Rule[] = [
  { regex: /.*?\*\//, token: "comment", pop: true },
  { regex: /.*/, token: "comment" },
];

// ── Makefile ───────────────────────────────────────────────────────────────
const MAKE: Modes = {
  start: [
    { regex: /#.*/, token: "comment" },
    { regex: /^\.[A-Z_]+\b/, token: "keyword", sol: true },
    {
      regex: /^(?:-?include|ifeq|ifneq|ifdef|ifndef|else|endif|define|endef|export|unexport|override|vpath|undefine)\b/,
      token: "controlKeyword", sol: true,
    },
    { regex: /^[A-Za-z_][\w.]*(?=\s*[:+?!]?=)/, token: "propertyName", sol: true },
    { regex: /^[^\s:=#][^:=#]*(?=::?(?!=))/, token: "typeName", sol: true },
    { regex: /\$[({][^)}]*[)}]/, token: "constName" },
    { regex: /\$[@<^?*%+|]/, token: "constName" },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    { regex: /'(?:[^\\']|\\.)*'?/, token: "string" },
    { regex: /[:=|;]+/, token: "operator" },
  ],
};

// ── Terraform / HCL ────────────────────────────────────────────────────────
const HCL: Modes = {
  start: [
    { regex: /(?:#|\/\/).*/, token: "comment" },
    { regex: /\/\*/, token: "comment", push: "comment" },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    {
      regex: /\b(?:resource|data|variable|output|locals|module|provider|terraform|provisioner|backend|dynamic|lifecycle|connection|check|import|removed|moved)\b/,
      token: "definitionKeyword",
    },
    { regex: /\b(?:for|in|if|else|endfor|endif|can|try)\b/, token: "controlKeyword" },
    { regex: /\b(?:true|false|null)\b/, token: "atom" },
    { regex: /\b\d+(?:\.\d+)?\b/, token: "number" },
    { regex: /\b(?:var|local|each|count|path|self)\b(?=\.)/, token: "constName" },
    { regex: /[A-Za-z_][\w-]*(?=\s*\()/, token: "fnName" },
    { regex: /[A-Za-z_][\w-]*(?=\s*=[^=])/, token: "propertyName" },
    { regex: /[{}[\]()]/, token: "bracket" },
    { regex: /[=!<>+\-*/%&|?:]+/, token: "operator" },
  ],
  comment: blockComment,
};

// ── GraphQL ────────────────────────────────────────────────────────────────
const GRAPHQL: Modes = {
  start: [
    { regex: /#.*/, token: "comment" },
    { regex: /"""/, token: "string", push: "blockString" },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    {
      regex: /\b(?:query|mutation|subscription|fragment|on|type|input|interface|union|enum|scalar|schema|directive|extend|implements|repeatable)\b/,
      token: "keyword",
    },
    { regex: /\b(?:true|false|null)\b/, token: "atom" },
    { regex: /-?\d+(?:\.\d+)?(?:[eE][-+]?\d+)?/, token: "number" },
    { regex: /\$[A-Za-z_]\w*/, token: "variableName" },
    { regex: /@[A-Za-z_]\w*/, token: "attributeName" },
    { regex: /[A-Za-z_]\w*(?=\s*:)/, token: "propertyName" },
    { regex: /\b[A-Z]\w*\b/, token: "typeName" },
    { regex: /[{}[\]()]/, token: "bracket" },
    { regex: /\.{3}|[!=|&]+/, token: "operator" },
  ],
  blockString: [
    { regex: /.*?"""/, token: "string", pop: true },
    { regex: /.*/, token: "string" },
  ],
};

// ── Prisma schema ──────────────────────────────────────────────────────────
const PRISMA: Modes = {
  start: [
    { regex: /\/\/.*/, token: "comment" },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    { regex: /\b(?:model|enum|datasource|generator|type|view)\b/, token: "definitionKeyword" },
    { regex: /@@?[A-Za-z_]\w*/, token: "attributeName" },
    {
      regex: /\b(?:String|Boolean|Int|BigInt|Float|Decimal|DateTime|Json|Bytes|Unsupported)\b/,
      token: "typeName",
    },
    { regex: /\b(?:true|false|null|env|auto|now|uuid|cuid|dbgenerated)\b/, token: "atom" },
    { regex: /\b\d+\b/, token: "number" },
    { regex: /[{}[\]()]/, token: "bracket" },
    { regex: /[?=]/, token: "operator" },
  ],
};

// ── Elixir ─────────────────────────────────────────────────────────────────
const ELIXIR: Modes = {
  start: [
    { regex: /#.*/, token: "comment" },
    { regex: /"""/, token: "string", push: "heredoc" },
    { regex: /~[a-zA-Z]?"(?:[^\\"]|\\.)*"?/, token: "string" },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    { regex: /'(?:[^\\']|\\.)*'?/, token: "string" },
    {
      regex: /\b(?:defmodule|defstruct|defprotocol|defimpl|defmacrop|defmacro|defguardp|defguard|defdelegate|defexception|defoverridable|defp|def)\b/,
      token: "definitionKeyword",
    },
    {
      regex: /\b(?:do|end|fn|if|unless|else|case|cond|with|for|receive|try|rescue|catch|after|raise|throw|when|and|or|not|in|import|require|alias|use|quote|unquote)\b/,
      token: "controlKeyword",
    },
    { regex: /\b(?:true|false|nil)\b/, token: "atom" },
    { regex: /:(?:[A-Za-z_]\w*[?!]?|"[^"]*")/, token: "atom" },
    { regex: /@[A-Za-z_]\w*/, token: "attributeName" },
    { regex: /\b[A-Z]\w*(?:\.[A-Z]\w*)*\b/, token: "typeName" },
    { regex: /\b\d[\d_]*(?:\.\d+)?\b/, token: "number" },
    { regex: /[a-z_]\w*[?!]?(?=\()/, token: "fnName" },
    { regex: /<-|->|\|>|=~|[+\-*/=<>!&|^]+/, token: "operator" },
    { regex: /[{}[\]()]/, token: "bracket" },
  ],
  heredoc: [
    { regex: /.*?"""/, token: "string", pop: true },
    { regex: /.*/, token: "string" },
  ],
};

// ── Nix ────────────────────────────────────────────────────────────────────
// `//` is Nix's attribute-set update operator, not a comment; only `#` and
// `/* … */` open comments here.
const NIX: Modes = {
  start: [
    { regex: /#.*/, token: "comment" },
    { regex: /\/\*/, token: "comment", push: "comment" },
    { regex: /''/, token: "string", push: "indentString" },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    { regex: /\b(?:let|in|rec|with|inherit|if|then|else|assert|or|import)\b/, token: "controlKeyword" },
    { regex: /\b(?:true|false|null)\b/, token: "atom" },
    { regex: /\b\d+\b/, token: "number" },
    { regex: /[A-Za-z_][\w'-]*(?=\s*=[^=])/, token: "propertyName" },
    { regex: /[{}[\]()]/, token: "bracket" },
    { regex: /\/\/|\+\+|->|[=:;,.?!@]/, token: "operator" },
  ],
  comment: blockComment,
  indentString: [
    { regex: /.*?''/, token: "string", pop: true },
    { regex: /.*/, token: "string" },
  ],
};

// ── Zig and Solidity (C-shaped, so the legacy clike tokenizer fits) ─────────
const ZIG = {
  name: "zig",
  keywords: words(
    "const var fn pub return break continue defer errdefer try catch async await suspend nosuspend resume " +
    "if else while for switch orelse unreachable and or comptime inline noinline export extern packed " +
    "threadlocal usingnamespace test errdefer align linksection callconv noalias allowzero volatile asm opaque",
  ),
  types: words(
    "i8 i16 i32 i64 i128 isize u8 u16 u32 u64 u128 usize f16 f32 f64 f80 f128 bool void noreturn type " +
    "anyerror anyframe anytype anyopaque comptime_int comptime_float c_int c_uint c_long c_ulong c_char " +
    "struct enum union error",
  ),
  atoms: words("true false null undefined"),
  blockKeywords: words("if else while for switch struct enum union fn test"),
  defKeywords: words("fn const var"),
  multiLineStrings: false,
};

const SOLIDITY = {
  name: "solidity",
  keywords: words(
    "pragma solidity import as using is contract interface library abstract function modifier event error " +
    "struct enum mapping returns return if else for while do break continue new delete emit require assert " +
    "revert try catch unchecked constructor receive fallback public private internal external pure view " +
    "payable virtual override memory storage calldata immutable constant indexed anonymous assembly",
  ),
  types: words(
    "address bool string bytes byte int uint int8 int16 int32 int64 int128 int256 uint8 uint16 uint32 " +
    "uint64 uint128 uint256 bytes1 bytes2 bytes4 bytes8 bytes16 bytes32 fixed ufixed",
  ),
  atoms: words("true false wei gwei ether seconds minutes hours days weeks msg block tx this super now"),
  blockKeywords: words("contract interface library function modifier struct enum if else for while do try catch"),
  defKeywords: words("contract interface library function modifier struct enum event"),
};

async function clike(conf: Parameters<typeof import("@codemirror/legacy-modes/mode/clike").clike>[0]) {
  const m = await import("@codemirror/legacy-modes/mode/clike");
  return stream(m.clike(conf));
}

/**
 * Extra language descriptions, in the same shape as `@codemirror/language-data`
 * so they can simply be appended to that list. Aliases are the language ids
 * from lib/fileLanguage.ts.
 */
export const EXTRA_LANGUAGES: LanguageDescription[] = [
  LanguageDescription.of({ name: "Makefile", alias: ["make", "makefile"], load: () => simple(MAKE) }),
  LanguageDescription.of({ name: "Terraform", alias: ["terraform", "hcl"], load: () => simple(HCL) }),
  LanguageDescription.of({ name: "GraphQL", alias: ["graphql", "gql"], load: () => simple(GRAPHQL) }),
  LanguageDescription.of({ name: "Prisma", alias: ["prisma"], load: () => simple(PRISMA) }),
  LanguageDescription.of({ name: "Elixir", alias: ["elixir"], load: () => simple(ELIXIR) }),
  LanguageDescription.of({ name: "Nix", alias: ["nix"], load: () => simple(NIX) }),
  LanguageDescription.of({ name: "Zig", alias: ["zig"], load: () => clike(ZIG) }),
  LanguageDescription.of({ name: "Solidity", alias: ["solidity"], load: () => clike(SOLIDITY) }),
  LanguageDescription.of({
    name: "Shader",
    alias: ["glsl", "hlsl", "wgsl", "gdshader"],
    load: () => import("@codemirror/legacy-modes/mode/clike").then((m) => stream(m.shader)),
  }),
];

/** The raw rule tables, exported for unit tests. */
export const TEST_MODES = { MAKE, HCL, GRAPHQL, PRISMA, ELIXIR, NIX };
