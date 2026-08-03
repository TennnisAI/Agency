import type { Extension } from "@codemirror/state";
import { LanguageDescription } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { EXTRA_LANGUAGES } from "./cmModes";
import { languageIdForPath } from "./fileLanguage";

// The editor used to hardcode seven languages, so anything else (Go, YAML,
// TOML, shell, SQL, C, …) opened as flat text. It now resolves against the
// full CodeMirror language-data set plus the extra grammars in cmModes.ts,
// keyed by the shared language ids in fileLanguage.ts. Every grammar is a
// dynamic import, so a file's language is fetched the first time one is
// opened rather than shipped in the initial bundle.
export const CM_REGISTRY = [...languages, ...EXTRA_LANGUAGES];

// Ids whose grammar lives under a different name in language-data, plus the
// ones we deliberately serve with a near neighbour: Svelte/Astro/Razor are
// HTML with an embedded script block, GDScript and Vyper are Python-shaped,
// and systemd/desktop units are INI files.
export const CM_LANGUAGE_NAMES: Record<string, string> = {
  "asm": "gas",
  "astro": "html",
  "blade": "html",
  "common-lisp": "common lisp",
  "desktop": "properties",
  "docker": "dockerfile",
  "dotenv": "properties",
  "emacs-lisp": "common lisp",
  "erb": "html",
  "fish": "shell",
  "fortran-fixed-form": "fortran",
  "fortran-free-form": "fortran",
  "gdscript": "python",
  "handlebars": "html",
  "jsonc": "json",
  "jsonl": "json",
  "luau": "lua",
  "mdx": "markdown",
  "objective-cpp": "objective-c++",
  "postcss": "css",
  "proto": "protobuf",
  "purescript": "haskell",
  "racket": "scheme",
  "raku": "perl",
  "razor": "html",
  "shellscript": "shell",
  "svelte": "html",
  "system-verilog": "systemverilog",
  "systemd": "properties",
  "twig": "html",
  "vb": "vb.net",
  "vyper": "python",
  "wasm": "webassembly",
};

/** The language description for a file, or null when nothing covers it. */
export function languageDescription(path: string, firstLine?: string): LanguageDescription | null {
  const id = languageIdForPath(path, firstLine);
  if (!id) return null;
  // Exact name/alias match only: language-data's fuzzy mode matches on
  // substrings, which would hand "typescript" to whatever declares "type".
  return LanguageDescription.matchLanguageName(CM_REGISTRY, CM_LANGUAGE_NAMES[id] ?? id, false);
}

/**
 * The grammar for a markdown fence tag (the "go" in ```go), or null.
 * Handing the whole registry to lang-markdown instead would use its fuzzy
 * name match, which resolves ```text to LaTeX (on the "tex" substring) and
 * misses short tags like ```py that are extensions rather than aliases.
 */
export function languageForFence(info: string): LanguageDescription | null {
  const tag = info.trim().split(/\s+/)[0].toLowerCase();
  if (!tag) return null;
  // A fence tag is usually an extension ("py") or a language id ("python").
  return languageDescription(`fence.${tag}`)
    ?? LanguageDescription.matchLanguageName(CM_REGISTRY, tag, false);
}

/**
 * Load the grammar for a file. Resolves to `[]` for file types no grammar
 * covers (and for a failed chunk load), which leaves the editor on plain text.
 * `firstLine` lets shebang scripts with no extension resolve too.
 */
export async function loadLanguage(path: string, firstLine?: string): Promise<Extension[]> {
  const desc = languageDescription(path, firstLine);
  if (!desc) return [];
  try {
    // LanguageDescription caches the loaded support, so reopening a file type
    // costs nothing after the first time.
    return [await desc.load()];
  } catch {
    return [];
  }
}
