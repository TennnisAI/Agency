// Grammars for the file types CodeMirror's language-data doesn't ship. The
// first group turns up constantly in real repos (Makefiles, Terraform,
// GraphQL, Zig, …); the second is the long tail that closes the gap with the
// diff view, which colors every id in lib/fileLanguage.ts through Shiki. With
// both groups every file type the app recognizes opens colored in the editor
// too, which is what lib/fileLanguage.test.ts now asserts.
//
// Each is a small stream tokenizer: comments, strings, numbers and keywords,
// which is all the file editor needs to stop rendering these as flat text.
// None is a parser — no indentation, folding or bracket matching — so where a
// grammar has to guess (a bare `"` in Vim script, a `/` in awk) it guesses the
// way the common case reads.
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

// ═══ The long tail ═════════════════════════════════════════════════════════
// Everything below exists so the editor colors the same file types the diff
// view does. These turn up rarely enough that the grammars stay deliberately
// thin: the shape of the file (headings, keys, directives) plus the usual
// comments, strings and numbers.

// ── Prose & markup ─────────────────────────────────────────────────────────

// reStructuredText. Section adornments are underlines of repeated punctuation,
// which is why the heading rule matches a whole line of one symbol.
const RST: Modes = {
  start: [
    { regex: /\.\.\s+[\w:-]+::.*/, token: "definitionKeyword", sol: true },
    { regex: /\.\.\s+_[^:]+:.*/, token: "link", sol: true },
    { regex: /\.\.(?:\s.*)?$/, token: "comment", sol: true },
    { regex: /[=\-`:'"~^_*+#<>]{4,}\s*$/, token: "heading", sol: true },
    { regex: /:[\w.+-]+:(?=`)/, token: "keyword" },
    { regex: /``[^`]*``/, token: "string" },
    { regex: /`[^`]*`_{0,2}/, token: "link" },
    { regex: /\*\*[^*]+\*\*/, token: "strong" },
    { regex: /\*[^*]+\*/, token: "emphasis" },
    { regex: /\|[\w .-]+\|/, token: "constName" },
    { regex: /:[\w .+-]+:(?=\s|$)/, token: "propertyName", sol: true },
    { regex: /\s*(?:[*+-]|\d+[.)])\s/, token: "operator", sol: true },
    { regex: /https?:\/\/\S+/, token: "url" },
  ],
};

const ASCIIDOC: Modes = {
  start: [
    { regex: /\/\/.*/, token: "comment", sol: true },
    { regex: /={1,6}\s.*/, token: "heading", sol: true },
    { regex: /(?:-{4}|={4}|\.{4}|\*{4}|\+{4}|_{4})\s*$/, token: "string", sol: true },
    { regex: /\.[^\s.].*/, token: "heading", sol: true },
    { regex: /:[\w!-]+:.*/, token: "propertyName", sol: true },
    { regex: /\[.*\]\s*$/, token: "attributeName", sol: true },
    { regex: /\s*(?:[*.]+|-)\s/, token: "operator", sol: true },
    { regex: /\{[\w-]+\}/, token: "constName" },
    { regex: /(?:https?|link|image|include|xref|kbd|btn):[^\s[]*(?:\[[^\]]*\])?/, token: "link" },
    { regex: /`[^`]+`/, token: "string" },
    { regex: /\*[^*\s][^*]*\*/, token: "strong" },
    { regex: /_[^_\s][^_]*_/, token: "emphasis" },
  ],
};

const TYPST: Modes = {
  start: [
    { regex: /\/\/.*/, token: "comment" },
    { regex: /\/\*/, token: "comment", push: "comment" },
    { regex: /=+\s.*/, token: "heading", sol: true },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    {
      regex: /#(?:let|set|show|import|include|if|else|for|while|return|break|continue|as|in|not|and|or|none|auto|true|false)\b/,
      token: "controlKeyword",
    },
    { regex: /#[A-Za-z][\w.-]*/, token: "fnName" },
    { regex: /@[\w:-]+/, token: "link" },
    { regex: /<[\w:-]+>/, token: "constName" },
    { regex: /\$[^$]*\$/, token: "string" },
    { regex: /\b\d+(?:\.\d+)?(?:pt|mm|cm|in|em|fr|deg|%)?/, token: "number" },
    { regex: /\*[^*\s][^*]*\*/, token: "strong" },
    { regex: /_[^_\s][^_]*_/, token: "emphasis" },
    { regex: /[{}[\]()]/, token: "bracket" },
  ],
  comment: blockComment,
};

// MediaWiki markup: templates, links, and the quote-count emphasis syntax.
const WIKITEXT: Modes = {
  start: [
    { regex: /<!--/, token: "comment", push: "htmlComment" },
    { regex: /=+[^=]+=+\s*$/, token: "heading", sol: true },
    { regex: /-{4,}\s*$/, token: "operator", sol: true },
    { regex: /[*#:;]+\s?/, token: "operator", sol: true },
    { regex: /\{\{[^{}|]*/, token: "definitionKeyword" },
    { regex: /\[\[[^\][|]*/, token: "link" },
    { regex: /\]\]|\}\}|\{\{|\|/, token: "bracket" },
    { regex: /'{5}[^']+'{5}/, token: "strong" },
    { regex: /'{3}[^']+'{3}/, token: "strong" },
    { regex: /''[^']+''/, token: "emphasis" },
    { regex: /<\/?[A-Za-z][\w-]*/, token: "tagName" },
    { regex: /https?:\/\/\S+/, token: "url" },
  ],
  htmlComment: [
    { regex: /.*?-->/, token: "comment", pop: true },
    { regex: /.*/, token: "comment" },
  ],
};

const BIBTEX: Modes = {
  start: [
    { regex: /%.*/, token: "comment" },
    { regex: /@[A-Za-z]+/, token: "definitionKeyword" },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    { regex: /[A-Za-z_][\w-]*(?=\s*=)/, token: "propertyName" },
    { regex: /\\[A-Za-z]+/, token: "escape" },
    { regex: /\b\d+\b/, token: "number" },
    { regex: /[{}]/, token: "bracket" },
    { regex: /[=,]/, token: "separator" },
    { regex: /[A-Za-z][\w:.-]*/, token: "variableName" },
  ],
};

// gettext catalogs: `#`-prefixed metadata lines and msgid/msgstr pairs.
const PO: Modes = {
  start: [
    { regex: /#[.:,|~]?.*/, token: "comment" },
    { regex: /\b(?:msgid|msgid_plural|msgstr|msgctxt|domain|obsolete|fuzzy)\b/, token: "keyword" },
    { regex: /\[\d+\]/, token: "number" },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
  ],
};

const HAML: Modes = {
  start: [
    { regex: /\s*(?:-#|\/).*/, token: "comment", sol: true },
    { regex: /!{3}.*/, token: "definitionKeyword", sol: true },
    { regex: /%[\w:-]+/, token: "tagName" },
    { regex: /#\{[^}]*\}/, token: "constName" },
    { regex: /[.#][\w-]+/, token: "attributeName" },
    { regex: /\s*[-=]\s/, token: "controlKeyword", sol: true },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    { regex: /:[\w-]+/, token: "atom" },
    { regex: /=>|[{}()[\]]/, token: "bracket" },
    { regex: /\b\d+\b/, token: "number" },
  ],
};

// ── Data & config ──────────────────────────────────────────────────────────

// Delimiter-separated values, comma or tab. Quoted fields read as strings and
// bare numeric fields as numbers, which is enough to see a ragged row.
const CSV: Modes = {
  start: [
    { regex: /"(?:[^"]|"")*"?/, token: "string" },
    { regex: /[,;\t]/, token: "separator" },
    { regex: /-?\d+(?:\.\d+)?(?=[,;\t]|$)/, token: "number" },
    { regex: /[^,;\t"]+/, token: "variableName" },
  ],
};

const HJSON: Modes = {
  start: [
    { regex: /(?:#|\/\/).*/, token: "comment" },
    { regex: /\/\*/, token: "comment", push: "comment" },
    { regex: /'''/, token: "string", push: "multiString" },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    { regex: /'(?:[^\\']|\\.)*'?/, token: "string" },
    { regex: /[A-Za-z_$][\w$-]*(?=\s*:)/, token: "propertyName" },
    { regex: /\b(?:true|false|null)\b/, token: "atom" },
    { regex: /-?\d+(?:\.\d+)?(?:[eE][-+]?\d+)?\b/, token: "number" },
    { regex: /[{}[\]]/, token: "bracket" },
    { regex: /[:,]/, token: "separator" },
  ],
  comment: blockComment,
  multiString: [
    { regex: /.*?'''/, token: "string", pop: true },
    { regex: /.*/, token: "string" },
  ],
};

const CUE: Modes = {
  start: [
    { regex: /\/\/.*/, token: "comment" },
    { regex: /"""/, token: "string", push: "multiString" },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    { regex: /\b(?:package|import|let|if|for|in|div|mod|quo|rem)\b/, token: "controlKeyword" },
    { regex: /\b(?:true|false|null)\b/, token: "atom" },
    { regex: /\b(?:string|bytes|bool|int|uint|float|number)\b/, token: "typeName" },
    { regex: /#[A-Za-z_]\w*/, token: "typeName" },
    { regex: /@[A-Za-z_]\w*/, token: "attributeName" },
    { regex: /[A-Za-z_$]\w*(?=\s*[?!]?\s*:)/, token: "propertyName" },
    { regex: /\b\d+(?:\.\d+)?(?:[KMGTP]i?)?\b/, token: "number" },
    { regex: /[{}[\]()]/, token: "bracket" },
    { regex: /[|&=!<>+\-*/?]+/, token: "operator" },
  ],
  multiString: [
    { regex: /.*?"""/, token: "string", pop: true },
    { regex: /.*/, token: "string" },
  ],
};

const JSONNET: Modes = {
  start: [
    { regex: /(?:\/\/|#).*/, token: "comment" },
    { regex: /\/\*/, token: "comment", push: "comment" },
    { regex: /\|{3}/, token: "string", push: "textBlock" },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    { regex: /'(?:[^\\']|\\.)*'?/, token: "string" },
    {
      regex: /\b(?:local|function|if|then|else|for|in|error|assert|import|importstr|importbin|tailstrict)\b/,
      token: "controlKeyword",
    },
    { regex: /\b(?:self|super|null|true|false)\b/, token: "atom" },
    { regex: /\bstd\.[a-zA-Z]\w*/, token: "fnName" },
    { regex: /\$/, token: "constName" },
    { regex: /[A-Za-z_]\w*(?=\s*(?:\+?::?)\s)/, token: "propertyName" },
    { regex: /\b\d+(?:\.\d+)?(?:[eE][-+]?\d+)?\b/, token: "number" },
    { regex: /[{}[\]()]/, token: "bracket" },
    { regex: /[+\-*/%=!<>&|:.]+/, token: "operator" },
  ],
  comment: blockComment,
  textBlock: [
    { regex: /.*?\|{3}/, token: "string", pop: true },
    { regex: /.*/, token: "string" },
  ],
};

// Windows registry export: `;` comments, `[HKEY…]` keys, `"name"=value`.
const REG: Modes = {
  start: [
    { regex: /;.*/, token: "comment" },
    { regex: /Windows Registry Editor.*/i, token: "definitionKeyword", sol: true },
    { regex: /\[-?[^\]]*\]?/, token: "typeName", sol: true },
    { regex: /"(?:[^\\"]|\\.)*"(?=\s*=)/, token: "propertyName" },
    { regex: /@(?=\s*=)/, token: "propertyName" },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    { regex: /\b(?:dword|qword|hex(?:\(\d\))?)\b/i, token: "keyword" },
    { regex: /[0-9a-fA-F]{2,}(?:,[0-9a-fA-F]{2})*/, token: "number" },
    { regex: /[=:,\\]/, token: "operator" },
  ],
};

const APACHE: Modes = {
  start: [
    { regex: /#.*/, token: "comment" },
    { regex: /<\/?[A-Za-z]\w*/, token: "tagName" },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    { regex: /\s*[A-Za-z]\w*/, token: "keyword", sol: true },
    { regex: /\b(?:on|off|all|none|any|granted|denied|valid-user|env|expr)\b/i, token: "atom" },
    { regex: /\$\{?\w+\}?|%\{[^}]*\}/, token: "constName" },
    { regex: /\b\d+\b/, token: "number" },
    { regex: /[>=!]/, token: "operator" },
  ],
};

const CODEOWNERS: Modes = {
  start: [
    { regex: /#.*/, token: "comment" },
    { regex: /[\w.+-]+@[\w.-]+/, token: "link" },
    { regex: /@[\w./-]+/, token: "constName" },
    { regex: /\S+/, token: "string" },
  ],
};

// ── Shell & editor scripting ───────────────────────────────────────────────

// Windows batch. `rem` and `::` only open a comment at the start of a command,
// which in practice means the start of a line.
const BAT: Modes = {
  start: [
    { regex: /\s*(?:rem\b|::).*/i, token: "comment", sol: true },
    { regex: /:[\w.-]+/, token: "typeName", sol: true },
    { regex: /@?\b(?:echo|set|setlocal|endlocal|call|goto|exit|shift|pause|start|title|pushd|popd|cd|del|copy|move|md|rd|type|find|findstr)\b/i, token: "keyword" },
    { regex: /\b(?:if|else|for|in|do|not|defined|exist|errorlevel|equ|neq|lss|leq|gtr|geq)\b/i, token: "controlKeyword" },
    { regex: /%~?\w*%?|![\w]+!/, token: "constName" },
    { regex: /"[^"]*"?/, token: "string" },
    { regex: /\s\/\w+/, token: "attributeName" },
    { regex: /\b\d+\b/, token: "number" },
    { regex: /[|&<>]+/, token: "operator" },
  ],
};

// Vim script. A leading `"` is a comment and a `"` anywhere else is a string;
// there is no way to tell them apart without parsing, so position decides.
const VIML: Modes = {
  start: [
    { regex: /\s*".*/, token: "comment", sol: true },
    {
      regex: /\b(?:function|endfunction|endfunc|func|return|if|elseif|else|endif|while|endwhile|for|endfor|try|catch|finally|endtry|throw|break|continue|call|execute|augroup|autocmd|command|source|runtime|finish|silent|normal|echo|echon|echomsg|echoerr|let|unlet|const|set|setlocal|setglobal|map|noremap|nnoremap|inoremap|vnoremap|xnoremap|nmap|imap|vmap|highlight|syntax)\b/,
      token: "controlKeyword",
    },
    { regex: /\b[gbwtslav]:\w+/, token: "constName" },
    { regex: /'(?:[^']|'')*'?/, token: "string" },
    { regex: /"(?:[^\\"]|\\.)*"/, token: "string" },
    { regex: /<[A-Za-z][\w-]*>/, token: "atom" },
    { regex: /&\w+/, token: "propertyName" },
    { regex: /\b\d+\b/, token: "number" },
    { regex: /[a-zA-Z_]\w*(?=\()/, token: "fnName" },
    { regex: /=~|!~|[=<>!+\-*/%.|]+/, token: "operator" },
  ],
};

const NUSHELL: Modes = {
  start: [
    { regex: /#.*/, token: "comment" },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    { regex: /'[^']*'?/, token: "string" },
    {
      regex: /\b(?:def|def-env|export|extern|module|use|source|alias|let|mut|const|if|else|match|for|while|loop|break|continue|return|do|try|catch|error|hide|overlay|register|where|each|par-each|from|to)\b/,
      token: "controlKeyword",
    },
    { regex: /\$[\w.]+/, token: "constName" },
    { regex: /--[\w-]+/, token: "attributeName" },
    { regex: /\b(?:true|false|null)\b/, token: "atom" },
    { regex: /\b\d+(?:\.\d+)?(?:[a-z]{1,3})?\b/, token: "number" },
    { regex: /\|/, token: "operator" },
    { regex: /[{}[\]()]/, token: "bracket" },
  ],
};

// awk. A `/` opens a regex unless a space follows it, which is the cheapest
// way to keep `a / b` from swallowing the rest of the line.
const AWK: Modes = {
  start: [
    { regex: /#.*/, token: "comment" },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    { regex: /\/(?:[^\\/\s]|\\.)(?:[^\\/]|\\.)*\//, token: "regexp" },
    { regex: /\b(?:BEGIN|END)\b/, token: "definitionKeyword" },
    {
      regex: /\b(?:function|if|else|while|for|do|break|continue|next|nextfile|exit|return|delete|in|getline|print|printf)\b/,
      token: "controlKeyword",
    },
    {
      regex: /\b(?:NR|NF|FS|OFS|ORS|RS|FILENAME|FNR|RSTART|RLENGTH|SUBSEP|CONVFMT|OFMT|ENVIRON|ARGC|ARGV)\b/,
      token: "constName",
    },
    {
      regex: /\b(?:length|substr|index|split|sub|gsub|match|sprintf|sin|cos|atan2|exp|log|sqrt|int|rand|srand|tolower|toupper|system|close|fflush)\b(?=\s*\()/,
      token: "fnName",
    },
    { regex: /\$\w+/, token: "constName" },
    { regex: /\b\d+(?:\.\d+)?\b/, token: "number" },
    { regex: /[{}[\]()]/, token: "bracket" },
    { regex: /[~=!<>+\-*/%^|&?:]+/, token: "operator" },
  ],
};

const APPLESCRIPT: Modes = {
  start: [
    { regex: /(?:--|#).*/, token: "comment" },
    { regex: /\(\*/, token: "comment", push: "nestedComment" },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    {
      regex: /\b(?:tell|end|if|then|else|repeat|while|until|times|with|without|try|on|error|considering|ignoring|script|property|global|local|set|copy|to|of|in|from|exit|return|continue)\b/,
      token: "controlKeyword",
    },
    {
      regex: /\b(?:application|document|window|folder|file|disk|text|word|paragraph|character|item|list|record|date|alias|POSIX)\b/,
      token: "typeName",
    },
    { regex: /\b(?:my|its|me|it|this)\b/, token: "self" },
    { regex: /\b(?:true|false|missing value)\b/, token: "atom" },
    { regex: /\b\d+(?:\.\d+)?\b/, token: "number" },
    { regex: /[=<>&+\-*/]+/, token: "operator" },
  ],
  nestedComment: [
    { regex: /.*?\*\)/, token: "comment", pop: true },
    { regex: /.*/, token: "comment" },
  ],
};

// ── Query & policy ─────────────────────────────────────────────────────────

const KUSTO: Modes = {
  start: [
    { regex: /\/\/.*/, token: "comment" },
    { regex: /"(?:[^\\"]|\\.)*"?|'(?:[^\\']|\\.)*'?/, token: "string" },
    {
      regex: /\b(?:let|where|project|project-away|project-rename|extend|summarize|by|join|union|order|sort|top|take|limit|distinct|count|render|mv-expand|mv-apply|parse|evaluate|invoke|range|datatable|print|on|kind|hint|as|materialize|make-series|serialize|partition|lookup|find|search|consume|externaldata|fork|facet|sample|getschema|asc|desc|between|has|contains|startswith|endswith|matches|and|or|not|in)\b/i,
      token: "controlKeyword",
    },
    {
      regex: /\b(?:ago|now|bin|floor|todatetime|totimespan|tostring|toint|tolong|toreal|strcat|strlen|substring|split|extract|replace|iff|iif|case|isnull|isnotnull|isempty|isnotempty|coalesce|dcount|countif|sumif|avg|sum|min|max|percentile|arg_max|arg_min|make_list|make_set|row_number|prev|next|format_datetime|datetime_diff|startofday|endofday)\b(?=\s*\()/,
      token: "fnName",
    },
    { regex: /\b(?:true|false|null)\b/, token: "atom" },
    { regex: /\b(?:string|int|long|real|bool|guid|decimal|dynamic|datetime|timespan)\b/, token: "typeName" },
    { regex: /\b\d+(?:\.\d+)?(?:ms|micro|tick|[dhms])?\b/, token: "number" },
    { regex: /\|/, token: "operator" },
    { regex: /[=!<>+\-*/%~]+/, token: "operator" },
    { regex: /[{}[\]()]/, token: "bracket" },
  ],
};

const CODEQL: Modes = {
  start: [
    { regex: /\/\/.*/, token: "comment" },
    { regex: /\/\*/, token: "comment", push: "comment" },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    {
      regex: /\b(?:import|module|class|extends|implements|predicate|from|where|select|as|order|by|asc|desc|exists|forall|forex|any|none|not|and|or|if|then|else|instanceof|abstract|cached|external|final|library|private|query|signature|additional|bindingset|language|pragma|transient|newtype)\b/,
      token: "controlKeyword",
    },
    { regex: /\b(?:this|result|super)\b/, token: "self" },
    { regex: /\b(?:boolean|int|float|string|date)\b/, token: "typeName" },
    { regex: /\b(?:true|false)\b/, token: "atom" },
    { regex: /\b\d+(?:\.\d+)?\b/, token: "number" },
    { regex: /\b[A-Z]\w*\b/, token: "typeName" },
    { regex: /[a-z_]\w*(?=\s*\()/, token: "fnName" },
    { regex: /[{}[\]()]/, token: "bracket" },
    { regex: /[=+\-*/<>!|&%]+/, token: "operator" },
  ],
  comment: blockComment,
};

// Policy rules — Rego files are mapped here too, and lex the same way.
const POLAR: Modes = {
  start: [
    { regex: /#.*/, token: "comment" },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    {
      regex: /\b(?:package|import|default|not|with|as|some|every|in|if|else|allow|deny|forall|matches|cut|debug|print|actor|resource|has|relations|roles|permissions|rule|declare)\b/,
      token: "controlKeyword",
    },
    { regex: /\b(?:true|false|null)\b/, token: "atom" },
    { regex: /\b(?:input|data)\b/, token: "constName" },
    { regex: /\b\d+(?:\.\d+)?\b/, token: "number" },
    { regex: /[A-Za-z_]\w*(?=\s*\()/, token: "fnName" },
    { regex: /:=|==|!=|>=|<=|[=<>+\-*/|&]+/, token: "operator" },
    { regex: /[{}[\]()]/, token: "bracket" },
    { regex: /[;,.]/, token: "separator" },
  ],
};

// ── Diagrams, logs & git metadata ──────────────────────────────────────────

const MERMAID: Modes = {
  start: [
    { regex: /%%.*/, token: "comment" },
    {
      regex: /\b(?:graph|flowchart|sequenceDiagram|classDiagram|stateDiagram(?:-v2)?|erDiagram|journey|gantt|pie|gitGraph|mindmap|timeline|quadrantChart|requirementDiagram|sankey-beta|xychart-beta|block-beta)\b/,
      token: "definitionKeyword",
    },
    {
      regex: /\b(?:subgraph|end|participant|actor|note|loop|alt|else|opt|par|and|rect|activate|deactivate|class|click|style|linkStyle|classDef|direction|section|title|dateFormat|axisFormat|state|link|autonumber|over|as)\b/,
      token: "controlKeyword",
    },
    { regex: /\b(?:TB|TD|BT|RL|LR)\b/, token: "atom" },
    { regex: /"[^"]*"?/, token: "string" },
    { regex: /\|[^|]*\|/, token: "string" },
    { regex: /-{2,3}>|-{2,3}|={2,3}>|={2,3}|-\.-+>|\.{2}>|<\|?--|\*--|o--|:::/, token: "operator" },
    { regex: /\b\d+(?:\.\d+)?\b/, token: "number" },
    { regex: /[[\]{}()]/, token: "bracket" },
  ],
};

// Application logs. Severity words carry the color; timestamps, bracketed
// context and quoted payloads carry the structure.
const LOG: Modes = {
  start: [
    { regex: /\b(?:FATAL|ERROR|SEVERE|CRITICAL|PANIC)\b/, token: "invalid" },
    { regex: /\b(?:WARN|WARNING)\b/, token: "atom" },
    { regex: /\b(?:INFO|NOTICE)\b/, token: "keyword" },
    { regex: /\b(?:DEBUG|TRACE|VERBOSE)\b/, token: "comment" },
    {
      regex: /\d{4}-\d{2}-\d{2}(?:[T ]\d{2}:\d{2}:\d{2}(?:[.,]\d+)?(?:Z|[+-]\d{2}:?\d{2})?)?/,
      token: "number",
    },
    { regex: /\d{2}:\d{2}:\d{2}(?:[.,]\d+)?/, token: "number" },
    { regex: /https?:\/\/\S+/, token: "url" },
    { regex: /"(?:[^\\"]|\\.)*"/, token: "string" },
    { regex: /'(?:[^\\']|\\.)*'/, token: "string" },
    { regex: /\[[^\]]*\]/, token: "propertyName" },
    { regex: /\b[\w.]*(?:Exception|Error)\b/, token: "invalid" },
    { regex: /\b(?:0x[0-9a-fA-F]+|\d+(?:\.\d+)?)\b/, token: "number" },
  ],
};

// COMMIT_EDITMSG. `#` only opens a comment at the start of a line, so `#42`
// mid-message stays an issue reference.
const GIT_COMMIT: Modes = {
  start: [
    { regex: /\s*#.*/, token: "comment", sol: true },
    {
      regex: /(?:build|chore|ci|docs|feat|fix|perf|refactor|revert|style|test)(?:\([^)]*\))?!?:/,
      token: "definitionKeyword", sol: true,
    },
    {
      regex: /(?:Co-authored-by|Signed-off-by|Reviewed-by|Acked-by|Tested-by|Fixes|Closes|Refs|BREAKING[- ]CHANGE):/i,
      token: "propertyName", sol: true,
    },
    { regex: /`[^`]+`/, token: "string" },
    { regex: /https?:\/\/\S+/, token: "url" },
    { regex: /\b[0-9a-f]{7,40}\b/, token: "number" },
    { regex: /#\d+/, token: "constName" },
  ],
};

const GIT_REBASE: Modes = {
  start: [
    { regex: /#.*/, token: "comment" },
    {
      regex: /(?:pick|reword|edit|squash|fixup|exec|break|drop|label|reset|merge|update-ref|[predsfxbltmu])\b/,
      token: "controlKeyword", sol: true,
    },
    { regex: /\b[0-9a-f]{7,40}\b/, token: "number" },
  ],
};

// ── Languages ──────────────────────────────────────────────────────────────

const NIM: Modes = {
  start: [
    { regex: /#\[/, token: "comment", push: "nestedComment" },
    { regex: /##?.*/, token: "comment" },
    { regex: /"""/, token: "string", push: "tripleString" },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    { regex: /'(?:[^\\']|\\.)'/, token: "character" },
    {
      regex: /\b(?:proc|func|method|iterator|template|macro|converter|type|const|let|var|object|enum|tuple|ref|ptr|distinct|concept)\b/,
      token: "definitionKeyword",
    },
    {
      regex: /\b(?:if|elif|else|case|of|when|while|for|in|notin|is|isnot|return|yield|discard|break|continue|block|try|except|finally|raise|defer|import|export|include|from|as|using|static|asm|bind|mixin|do|and|or|not|xor|shl|shr|div|mod|cast|addr)\b/,
      token: "controlKeyword",
    },
    {
      regex: /\b(?:int|int8|int16|int32|int64|uint|uint8|uint16|uint32|uint64|float|float32|float64|bool|char|string|cstring|pointer|seq|array|openArray|set|void|auto|Natural|Positive)\b/,
      token: "typeName",
    },
    { regex: /\b(?:true|false|nil)\b/, token: "atom" },
    { regex: /\{\.[^}]*\.?\}?/, token: "attributeName" },
    { regex: /\b\d[\d_]*(?:\.\d+)?(?:'?[iuf]\d+)?\b/, token: "number" },
    { regex: /\b[A-Z]\w*\b/, token: "typeName" },
    { regex: /[a-zA-Z_]\w*(?=\()/, token: "fnName" },
    { regex: /[{}[\]()]/, token: "bracket" },
    { regex: /[=+\-*/<>@$~&%|!?^.:\\]+/, token: "operator" },
  ],
  nestedComment: [
    { regex: /.*?]#/, token: "comment", pop: true },
    { regex: /.*/, token: "comment" },
  ],
  tripleString: [
    { regex: /.*?"""/, token: "string", pop: true },
    { regex: /.*/, token: "string" },
  ],
};

const GLEAM: Modes = {
  start: [
    { regex: /\/{2,4}.*/, token: "comment" },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    { regex: /\b(?:pub|fn|let|const|type|import|opaque|external)\b/, token: "definitionKeyword" },
    { regex: /\b(?:case|if|else|as|use|assert|todo|panic|try)\b/, token: "controlKeyword" },
    { regex: /\b(?:True|False|Nil|Ok|Error)\b/, token: "atom" },
    { regex: /@[a-z_]\w*/, token: "attributeName" },
    { regex: /\b[A-Z]\w*\b/, token: "typeName" },
    { regex: /\b\d[\d_]*(?:\.\d+)?\b/, token: "number" },
    { regex: /[a-z_]\w*(?=\()/, token: "fnName" },
    { regex: /<-|->|\|>|[=+\-*/<>!&|.:]+/, token: "operator" },
    { regex: /[{}[\]()]/, token: "bracket" },
  ],
};

const ADA: Modes = {
  start: [
    { regex: /--.*/, token: "comment" },
    { regex: /"(?:[^"]|"")*"?/, token: "string" },
    { regex: /'.'/, token: "character" },
    {
      regex: /\b(?:package|procedure|function|type|subtype|task|entry|protected|generic|body|is|new|with|use|renames|separate)\b/i,
      token: "definitionKeyword",
    },
    {
      regex: /\b(?:begin|end|declare|if|then|elsif|else|case|when|loop|while|for|exit|return|goto|others|and|or|xor|not|in|out|access|constant|abstract|aliased|all|array|at|delta|digits|do|exception|raise|range|record|rem|abs|mod|of|pragma|private|requeue|reverse|select|delay|until|terminate|accept|abort|limited|interface|overriding|synchronized|tagged|some)\b/i,
      token: "controlKeyword",
    },
    {
      regex: /\b(?:Integer|Natural|Positive|Float|Boolean|Character|String|Duration|Long_Integer|Long_Float)\b/i,
      token: "typeName",
    },
    { regex: /\b(?:True|False|null)\b/i, token: "atom" },
    { regex: /\b\d+(?:#[0-9A-Fa-f_]+#|(?:\.\d+)?(?:[eE][-+]?\d+)?)/, token: "number" },
    { regex: /'[A-Za-z_]\w*/, token: "attributeName" },
    { regex: /:=|=>|\.\.|[<>=+\-*/&|]+/, token: "operator" },
    { regex: /[()]/, token: "bracket" },
  ],
};

// QML: JavaScript expressions inside a tree of `Type { property: value }`.
const QML: Modes = {
  start: [
    { regex: /\/\/.*/, token: "comment" },
    { regex: /\/\*/, token: "comment", push: "comment" },
    { regex: /"(?:[^\\"]|\\.)*"?/, token: "string" },
    { regex: /'(?:[^\\']|\\.)*'?/, token: "string" },
    {
      regex: /\b(?:import|pragma|as|property|readonly|default|required|signal|alias|component|enum|on)\b/,
      token: "definitionKeyword",
    },
    {
      regex: /\b(?:function|var|let|const|if|else|for|while|do|switch|case|return|break|continue|new|delete|typeof|throw|try|catch|finally)\b/,
      token: "controlKeyword",
    },
    {
      regex: /\b(?:int|real|double|bool|string|url|color|date|point|rect|size|variant|list|parent)\b/,
      token: "typeName",
    },
    { regex: /\b(?:true|false|null|undefined)\b/, token: "atom" },
    { regex: /\b[A-Z]\w*(?=\s*\{)/, token: "typeName" },
    { regex: /\b[A-Za-z_]\w*(?=\s*:)/, token: "propertyName" },
    { regex: /\b\d+(?:\.\d+)?\b/, token: "number" },
    { regex: /[a-z_]\w*(?=\()/, token: "fnName" },
    { regex: /[{}[\]()]/, token: "bracket" },
    { regex: /[=+\-*/<>!&|?:.]+/, token: "operator" },
  ],
  comment: blockComment,
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

// Cairo and Move are the other two the clike tokenizer fits: both are
// Rust-shaped, down to `//` comments and `fn`/`struct` declarations.
const CAIRO = {
  name: "cairo",
  keywords: words(
    "fn let mut const struct enum trait impl mod use pub extern type match if else loop while for " +
    "return break continue ref in as of dyn implicits nopanic assert super crate where",
  ),
  types: words(
    "felt252 u8 u16 u32 u64 u128 u256 usize i8 i16 i32 i64 i128 bool ByteArray Array Span Option " +
    "Result ContractAddress ClassHash",
  ),
  atoms: words("true false self"),
  blockKeywords: words("if else while for loop match struct enum impl trait mod fn"),
  defKeywords: words("fn struct enum trait impl mod type const"),
};

const MOVE = {
  name: "move",
  keywords: words(
    "module script fun public entry native struct has copy drop store key let mut const use friend " +
    "if else while loop return abort break continue move spec schema invariant ensures aborts_if " +
    "as acquires phantom",
  ),
  types: words("u8 u16 u32 u64 u128 u256 bool address vector signer"),
  atoms: words("true false"),
  blockKeywords: words("if else while loop struct module script fun spec"),
  defKeywords: words("fun struct module const"),
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

  // The long tail, in the same order as the rule tables above.
  LanguageDescription.of({ name: "reStructuredText", alias: ["rst"], load: () => simple(RST) }),
  LanguageDescription.of({ name: "AsciiDoc", alias: ["asciidoc", "adoc"], load: () => simple(ASCIIDOC) }),
  LanguageDescription.of({ name: "Typst", alias: ["typst", "typ"], load: () => simple(TYPST) }),
  LanguageDescription.of({ name: "Wikitext", alias: ["wikitext", "wiki"], load: () => simple(WIKITEXT) }),
  LanguageDescription.of({ name: "BibTeX", alias: ["bibtex", "bib"], load: () => simple(BIBTEX) }),
  LanguageDescription.of({ name: "Gettext", alias: ["po", "pot", "gettext"], load: () => simple(PO) }),
  LanguageDescription.of({ name: "Haml", alias: ["haml"], load: () => simple(HAML) }),
  LanguageDescription.of({ name: "CSV", alias: ["csv", "tsv"], load: () => simple(CSV) }),
  LanguageDescription.of({ name: "Hjson", alias: ["hjson"], load: () => simple(HJSON) }),
  LanguageDescription.of({ name: "CUE", alias: ["cue"], load: () => simple(CUE) }),
  LanguageDescription.of({ name: "Jsonnet", alias: ["jsonnet", "libsonnet"], load: () => simple(JSONNET) }),
  LanguageDescription.of({ name: "Registry", alias: ["reg"], load: () => simple(REG) }),
  LanguageDescription.of({ name: "Apache", alias: ["apache"], load: () => simple(APACHE) }),
  LanguageDescription.of({ name: "CODEOWNERS", alias: ["codeowners"], load: () => simple(CODEOWNERS) }),
  LanguageDescription.of({ name: "Batch", alias: ["bat", "cmd", "batch"], load: () => simple(BAT) }),
  LanguageDescription.of({ name: "Vim script", alias: ["viml", "vim"], load: () => simple(VIML) }),
  LanguageDescription.of({ name: "Nushell", alias: ["nushell", "nu"], load: () => simple(NUSHELL) }),
  LanguageDescription.of({ name: "Awk", alias: ["awk"], load: () => simple(AWK) }),
  LanguageDescription.of({ name: "AppleScript", alias: ["applescript"], load: () => simple(APPLESCRIPT) }),
  LanguageDescription.of({ name: "Kusto", alias: ["kusto", "kql"], load: () => simple(KUSTO) }),
  LanguageDescription.of({ name: "CodeQL", alias: ["codeql", "ql"], load: () => simple(CODEQL) }),
  LanguageDescription.of({ name: "Polar", alias: ["polar", "rego"], load: () => simple(POLAR) }),
  LanguageDescription.of({ name: "Mermaid", alias: ["mermaid", "mmd"], load: () => simple(MERMAID) }),
  LanguageDescription.of({ name: "Log", alias: ["log"], load: () => simple(LOG) }),
  LanguageDescription.of({ name: "Git commit", alias: ["git-commit"], load: () => simple(GIT_COMMIT) }),
  LanguageDescription.of({ name: "Git rebase", alias: ["git-rebase"], load: () => simple(GIT_REBASE) }),
  LanguageDescription.of({ name: "Nim", alias: ["nim"], load: () => simple(NIM) }),
  LanguageDescription.of({ name: "Gleam", alias: ["gleam"], load: () => simple(GLEAM) }),
  LanguageDescription.of({ name: "Ada", alias: ["ada"], load: () => simple(ADA) }),
  LanguageDescription.of({ name: "QML", alias: ["qml"], load: () => simple(QML) }),
  LanguageDescription.of({ name: "Cairo", alias: ["cairo"], load: () => clike(CAIRO) }),
  LanguageDescription.of({ name: "Move", alias: ["move"], load: () => clike(MOVE) }),
];

/** The raw rule tables, exported for unit tests. */
export const TEST_MODES = {
  MAKE, HCL, GRAPHQL, PRISMA, ELIXIR, NIX,
  RST, ASCIIDOC, TYPST, WIKITEXT, BIBTEX, PO, HAML,
  CSV, HJSON, CUE, JSONNET, REG, APACHE, CODEOWNERS,
  BAT, VIML, NUSHELL, AWK, APPLESCRIPT,
  KUSTO, CODEQL, POLAR,
  MERMAID, LOG, GIT_COMMIT, GIT_REBASE,
  NIM, GLEAM, ADA, QML,
};
