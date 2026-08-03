// Map a file name to a tree icon shape + a theme-accent color var. Colors are
// drawn from the active theme's tokens (theme.css) rather than fixed hex, so
// file-type icons stay in-palette and adapt to light ("Paperback") and dark
// themes automatically. Modeled on lib/fileLanguage.ts.

import { languageIdForPath } from "./fileLanguage";

export type FileIconKind =
  | "code" // languages: ts/js/py/rs/go/…
  | "braces" // structured data: json/yaml/toml
  | "markup" // html/xml/svg-as-markup
  | "style" // css/scss/less
  | "doc" // md/txt/prose
  | "image" // png/jpg/gif/webp
  | "audio" // mp3/wav/flac/…
  | "video" // mp4/mov/webm/…
  | "pdf" // pdf
  | "archive" // zip/tar/gz/…
  | "font" // ttf/otf/woff/…
  | "binary" // db/wasm/exe/… (opaque data)
  | "lock" // secrets & lockfiles: .env/*.pem/*.lock
  | "config" // dotfiles & build config
  | "folder" // directories (rendered by FileTree, not mapped by extension)
  | "file"; // unknown / generic

export interface FileIconSpec {
  kind: FileIconKind;
  /** CSS color, always a `var(--token)` from the theme palette. */
  color: string;
}

const DEFAULT: FileIconSpec = { kind: "file", color: "var(--sub1)" };

// Whole-name matches (checked before extension). Keyed lowercase.
const BY_NAME: Record<string, FileIconSpec> = {
  "package.json": { kind: "braces", color: "var(--red)" },
  "tsconfig.json": { kind: "braces", color: "var(--blue)" },
  "dockerfile": { kind: "config", color: "var(--blue)" },
  "makefile": { kind: "config", color: "var(--peach)" },
  ".gitignore": { kind: "config", color: "var(--sub1)" },
  ".gitattributes": { kind: "config", color: "var(--sub1)" },
  ".editorconfig": { kind: "config", color: "var(--sub1)" },
  ".npmrc": { kind: "config", color: "var(--sub1)" },
  "cargo.lock": { kind: "lock", color: "var(--sub1)" },
  "package-lock.json": { kind: "lock", color: "var(--sub1)" },
  "pnpm-lock.yaml": { kind: "lock", color: "var(--sub1)" },
  "yarn.lock": { kind: "lock", color: "var(--sub1)" },
};

// Extension → spec. Keep in step with fileLanguage.ts where they overlap; any
// language that file knows but this one doesn't still gets a code icon (see
// the fallback in fileIcon()), so only add an entry to pick a color.
const BY_EXT: Record<string, FileIconSpec> = {
  ts: { kind: "code", color: "var(--blue)" },
  tsx: { kind: "code", color: "var(--blue)" },
  js: { kind: "code", color: "var(--yellow)" },
  jsx: { kind: "code", color: "var(--yellow)" },
  mjs: { kind: "code", color: "var(--yellow)" },
  cjs: { kind: "code", color: "var(--yellow)" },
  py: { kind: "code", color: "var(--green)" },
  rs: { kind: "code", color: "var(--red)" },
  go: { kind: "code", color: "var(--teal)" },
  rb: { kind: "code", color: "var(--red)" },
  java: { kind: "code", color: "var(--peach)" },
  c: { kind: "code", color: "var(--blue)" },
  h: { kind: "code", color: "var(--blue)" },
  cpp: { kind: "code", color: "var(--blue)" },
  swift: { kind: "code", color: "var(--peach)" },
  php: { kind: "code", color: "var(--mauve)" },
  kt: { kind: "code", color: "var(--mauve)" },
  kts: { kind: "code", color: "var(--mauve)" },
  cs: { kind: "code", color: "var(--green)" },
  scala: { kind: "code", color: "var(--red)" },
  ex: { kind: "code", color: "var(--mauve)" },
  exs: { kind: "code", color: "var(--mauve)" },
  lua: { kind: "code", color: "var(--blue)" },
  zig: { kind: "code", color: "var(--peach)" },
  dart: { kind: "code", color: "var(--teal)" },
  hs: { kind: "code", color: "var(--mauve)" },
  sql: { kind: "code", color: "var(--peach)" },
  graphql: { kind: "code", color: "var(--pink)" },
  gql: { kind: "code", color: "var(--pink)" },
  prisma: { kind: "code", color: "var(--teal)" },
  tf: { kind: "code", color: "var(--mauve)" },
  tfvars: { kind: "code", color: "var(--mauve)" },
  nix: { kind: "code", color: "var(--blue)" },

  json: { kind: "braces", color: "var(--peach)" },
  jsonc: { kind: "braces", color: "var(--peach)" },
  yaml: { kind: "braces", color: "var(--peach)" },
  yml: { kind: "braces", color: "var(--peach)" },
  toml: { kind: "braces", color: "var(--peach)" },

  html: { kind: "markup", color: "var(--peach)" },
  htm: { kind: "markup", color: "var(--peach)" },
  xml: { kind: "markup", color: "var(--peach)" },
  vue: { kind: "markup", color: "var(--green)" },
  svelte: { kind: "markup", color: "var(--peach)" },

  css: { kind: "style", color: "var(--mauve)" },
  scss: { kind: "style", color: "var(--mauve)" },
  sass: { kind: "style", color: "var(--mauve)" },
  less: { kind: "style", color: "var(--mauve)" },

  md: { kind: "doc", color: "var(--green)" },
  markdown: { kind: "doc", color: "var(--green)" },
  mdx: { kind: "doc", color: "var(--green)" },
  txt: { kind: "doc", color: "var(--sub1)" },
  rst: { kind: "doc", color: "var(--green)" },
  log: { kind: "doc", color: "var(--sub1)" },
  diff: { kind: "doc", color: "var(--sub1)" },
  patch: { kind: "doc", color: "var(--sub1)" },
  csv: { kind: "braces", color: "var(--green)" },
  tsv: { kind: "braces", color: "var(--green)" },

  png: { kind: "image", color: "var(--teal)" },
  jpg: { kind: "image", color: "var(--teal)" },
  jpeg: { kind: "image", color: "var(--teal)" },
  gif: { kind: "image", color: "var(--teal)" },
  webp: { kind: "image", color: "var(--teal)" },
  bmp: { kind: "image", color: "var(--teal)" },
  avif: { kind: "image", color: "var(--teal)" },
  svg: { kind: "image", color: "var(--teal)" },
  ico: { kind: "image", color: "var(--teal)" },

  mp3: { kind: "audio", color: "var(--mauve)" },
  wav: { kind: "audio", color: "var(--mauve)" },
  flac: { kind: "audio", color: "var(--mauve)" },
  aac: { kind: "audio", color: "var(--mauve)" },
  ogg: { kind: "audio", color: "var(--mauve)" },
  oga: { kind: "audio", color: "var(--mauve)" },
  m4a: { kind: "audio", color: "var(--mauve)" },
  opus: { kind: "audio", color: "var(--mauve)" },
  wma: { kind: "audio", color: "var(--mauve)" },
  aiff: { kind: "audio", color: "var(--mauve)" },
  aif: { kind: "audio", color: "var(--mauve)" },

  mp4: { kind: "video", color: "var(--pink)" },
  m4v: { kind: "video", color: "var(--pink)" },
  mov: { kind: "video", color: "var(--pink)" },
  webm: { kind: "video", color: "var(--pink)" },
  mkv: { kind: "video", color: "var(--pink)" },
  avi: { kind: "video", color: "var(--pink)" },
  wmv: { kind: "video", color: "var(--pink)" },
  flv: { kind: "video", color: "var(--pink)" },

  pdf: { kind: "pdf", color: "var(--red)" },

  zip: { kind: "archive", color: "var(--yellow)" },
  tar: { kind: "archive", color: "var(--yellow)" },
  gz: { kind: "archive", color: "var(--yellow)" },
  tgz: { kind: "archive", color: "var(--yellow)" },
  bz2: { kind: "archive", color: "var(--yellow)" },
  xz: { kind: "archive", color: "var(--yellow)" },
  zst: { kind: "archive", color: "var(--yellow)" },
  rar: { kind: "archive", color: "var(--yellow)" },
  "7z": { kind: "archive", color: "var(--yellow)" },

  ttf: { kind: "font", color: "var(--peach)" },
  otf: { kind: "font", color: "var(--peach)" },
  woff: { kind: "font", color: "var(--peach)" },
  woff2: { kind: "font", color: "var(--peach)" },
  eot: { kind: "font", color: "var(--peach)" },

  db: { kind: "binary", color: "var(--sub1)" },
  sqlite: { kind: "binary", color: "var(--sub1)" },
  sqlite3: { kind: "binary", color: "var(--sub1)" },
  wasm: { kind: "binary", color: "var(--sub1)" },
  exe: { kind: "binary", color: "var(--sub1)" },
  dll: { kind: "binary", color: "var(--sub1)" },
  so: { kind: "binary", color: "var(--sub1)" },
  dylib: { kind: "binary", color: "var(--sub1)" },
  bin: { kind: "binary", color: "var(--sub1)" },
  dat: { kind: "binary", color: "var(--sub1)" },

  sh: { kind: "config", color: "var(--green)" },
  bash: { kind: "config", color: "var(--green)" },
  zsh: { kind: "config", color: "var(--green)" },

  pem: { kind: "lock", color: "var(--yellow)" },
  key: { kind: "lock", color: "var(--yellow)" },
  lock: { kind: "lock", color: "var(--sub1)" },
};

export function fileIcon(name: string): FileIconSpec {
  const lower = name.toLowerCase();
  // Own-property checks only, so a file named "constructor" can't inherit a
  // match from Object.prototype.
  if (Object.prototype.hasOwnProperty.call(BY_NAME, lower)) return BY_NAME[lower];
  // .env, .env.local, .env.production, … are secrets.
  if (lower === ".env" || lower.startsWith(".env.")) {
    return { kind: "lock", color: "var(--yellow)" };
  }
  const ext = lower.includes(".") ? lower.split(".").pop()! : "";
  if (Object.prototype.hasOwnProperty.call(BY_EXT, ext)) return BY_EXT[ext];
  // Source the editor can highlight but that has no color of its own above:
  // still a code file, just an uncolored one. Keeps the tree honest as the
  // language list in fileLanguage.ts grows.
  if (languageIdForPath(lower)) return { kind: "code", color: "var(--sub1)" };
  return DEFAULT;
}
