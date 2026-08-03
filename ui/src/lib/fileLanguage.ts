// One place that answers "what language is this file?", modeled on VS Code's
// own language contributions (filename, then extension, then shebang). The ids
// are TextMate/VS Code ids, which is also exactly what Shiki's grammar bundle
// is keyed by — so the diff highlighter (components/git/highlight.ts) can feed
// an id straight to Shiki, and the file editor (lib/cmLanguage.ts) maps the
// same id onto a CodeMirror grammar. Both views therefore recognize the same
// file types instead of each keeping its own short list.

// Whole-name matches, checked first (lowercased). VS Code ships these as
// `filenames` contributions; the rest of the world calls them "that file with
// no extension you still want colored".
const BY_NAME: Record<string, string> = {
  "dockerfile": "docker",
  "containerfile": "docker",
  "makefile": "make",
  "gnumakefile": "make",
  "justfile": "make",
  "cmakelists.txt": "cmake",
  "gemfile": "ruby",
  "rakefile": "ruby",
  "podfile": "ruby",
  "brewfile": "ruby",
  "vagrantfile": "ruby",
  "fastfile": "ruby",
  "appfile": "ruby",
  "jenkinsfile": "groovy",
  "build.bazel": "python", // Starlark is a Python dialect
  "workspace.bazel": "python",
  "pkgbuild": "shellscript",
  "codeowners": "codeowners",
  "commit_editmsg": "git-commit",
  "merge_msg": "git-commit",
  "git-rebase-todo": "git-rebase",
  "nginx.conf": "nginx",
  ".htaccess": "apache",
  ".babelrc": "jsonc",
  ".eslintrc": "jsonc",
  ".prettierrc": "jsonc",
  ".swcrc": "jsonc",
  ".gitignore": "ini",
  ".dockerignore": "ini",
  ".npmignore": "ini",
  ".eslintignore": "ini",
  ".prettierignore": "ini",
  ".gitattributes": "ini",
  ".gitmodules": "ini",
  ".gitconfig": "ini",
  ".editorconfig": "ini",
  ".npmrc": "ini",
  ".curlrc": "ini",
  ".inputrc": "ini",
  ".flake8": "ini",
  "setup.cfg": "ini",
  "tox.ini": "ini",
  "pytest.ini": "ini",
  ".bashrc": "shellscript",
  ".bash_profile": "shellscript",
  ".bash_aliases": "shellscript",
  ".bash_logout": "shellscript",
  ".profile": "shellscript",
  ".zshrc": "shellscript",
  ".zshenv": "shellscript",
  ".zprofile": "shellscript",
  ".zlogin": "shellscript",
  ".vimrc": "viml",
  ".gvimrc": "viml",
};

// Extension → language id. Where a file type has no grammar of its own we
// point it at the closest one that lexes the same way (Svelte/Astro at HTML,
// Starlark at Python), which colors comments, strings and numbers correctly
// even when a keyword or two is missed.
const BY_EXT: Record<string, string> = {
  // ── Web ──────────────────────────────────────────────────────────────
  js: "javascript", mjs: "javascript", cjs: "javascript",
  jsx: "jsx",
  ts: "typescript", mts: "typescript", cts: "typescript",
  tsx: "tsx",
  json: "json", map: "json", webmanifest: "json", geojson: "json", topojson: "json",
  jsonc: "jsonc", json5: "json5", jsonl: "jsonl", ndjson: "jsonl",
  html: "html", htm: "html", xhtml: "html",
  vue: "vue", svelte: "svelte", astro: "astro", marko: "marko",
  hbs: "handlebars", handlebars: "handlebars", mustache: "handlebars",
  ejs: "html", njk: "jinja", jinja: "jinja", jinja2: "jinja", j2: "jinja",
  liquid: "liquid", twig: "twig", erb: "erb", haml: "haml", pug: "pug", jade: "pug",
  css: "css", scss: "scss", sass: "sass", less: "less",
  styl: "stylus", pcss: "postcss", postcss: "postcss",

  // ── Systems ──────────────────────────────────────────────────────────
  c: "c", h: "c", ino: "c",
  cpp: "cpp", "c++": "cpp", cc: "cpp", cxx: "cpp",
  hpp: "cpp", "h++": "cpp", hh: "cpp", hxx: "cpp", ipp: "cpp", tpp: "cpp",
  cu: "cpp", cuh: "cpp", metal: "cpp",
  m: "objective-c", mm: "objective-cpp",
  rs: "rust", go: "go", zig: "zig", nim: "nim", d: "d", v: "verilog",
  sv: "system-verilog", svh: "system-verilog", vhd: "vhdl", vhdl: "vhdl",
  asm: "asm", s: "asm",
  wat: "wasm", wast: "wasm",
  glsl: "glsl", vert: "glsl", frag: "glsl", geom: "glsl", comp: "glsl",
  hlsl: "hlsl", wgsl: "wgsl", gdshader: "gdshader",

  // ── JVM / .NET ───────────────────────────────────────────────────────
  java: "java", kt: "kotlin", kts: "kotlin",
  scala: "scala", sc: "scala", groovy: "groovy", gradle: "groovy",
  clj: "clojure", cljc: "clojure", cljs: "clojure", edn: "clojure",
  cs: "csharp", csx: "csharp", fs: "fsharp", fsi: "fsharp", fsx: "fsharp",
  vb: "vb", vbs: "vb", razor: "razor", cshtml: "razor",

  // ── Scripting & applications ─────────────────────────────────────────
  py: "python", pyw: "python", pyi: "python", pyx: "python", bzl: "python",
  rb: "ruby", rake: "ruby", gemspec: "ruby", ru: "ruby",
  pl: "perl", pm: "perl", raku: "raku", p6: "raku",
  php: "php", phtml: "php", php3: "php", php4: "php", php5: "php", php7: "php",
  "blade.php": "blade",
  lua: "lua", luau: "luau", tcl: "tcl", r: "r", rmd: "r", jl: "julia",
  sh: "shellscript", bash: "shellscript", zsh: "shellscript", ksh: "shellscript",
  ash: "shellscript", bats: "shellscript", ebuild: "shellscript",
  fish: "fish", nu: "nushell",
  ps1: "powershell", psm1: "powershell", psd1: "powershell",
  bat: "bat", cmd: "bat",
  vim: "viml", vimrc: "viml",
  el: "emacs-lisp", lisp: "common-lisp", cl: "common-lisp", lsp: "common-lisp",
  scm: "scheme", ss: "scheme", rkt: "racket",
  ex: "elixir", exs: "elixir", heex: "elixir", eex: "elixir",
  erl: "erlang", hrl: "erlang",
  hs: "haskell", lhs: "haskell", elm: "elm", purs: "purescript",
  ml: "ocaml", mli: "ocaml",
  swift: "swift", dart: "dart", cr: "crystal", gleam: "gleam", hx: "haxe",
  pas: "pascal", adb: "ada", ads: "ada",
  f: "fortran-fixed-form", for: "fortran-fixed-form", f77: "fortran-fixed-form",
  f90: "fortran-free-form", f95: "fortran-free-form", f03: "fortran-free-form",
  cob: "cobol", cbl: "cobol",
  sol: "solidity", vy: "vyper", move: "move", cairo: "cairo",
  applescript: "applescript", scpt: "applescript",
  gd: "gdscript", qml: "qml", coffee: "coffee",

  // ── Data, config, schemas ────────────────────────────────────────────
  yaml: "yaml", yml: "yaml",
  toml: "toml",
  ini: "ini", cfg: "ini", conf: "ini", properties: "ini", desktop: "desktop",
  env: "dotenv",
  xml: "xml", xsd: "xml", xsl: "xml", xslt: "xml", svg: "xml", plist: "xml",
  xaml: "xml", csproj: "xml", vbproj: "xml", fsproj: "xml", wsdl: "xml",
  rss: "xml", atom: "xml", dtd: "xml", storyboard: "xml", xib: "xml",
  csv: "csv", tsv: "tsv",
  sql: "sql", prisma: "prisma", graphql: "graphql", gql: "graphql",
  proto: "proto", thrift: "proto",
  tf: "terraform", tfvars: "terraform", hcl: "hcl",
  nix: "nix", cue: "cue", jsonnet: "jsonnet", libsonnet: "jsonnet", hjson: "hjson",
  cmake: "cmake", "cmake.in": "cmake", mk: "make", mak: "make", dockerfile: "docker",
  service: "systemd", socket: "systemd", timer: "systemd",
  reg: "reg", ql: "codeql", kql: "kusto", rego: "polar",

  // ── Prose & other ────────────────────────────────────────────────────
  md: "markdown", markdown: "markdown", mdown: "markdown", mkd: "markdown",
  mdx: "mdx", rst: "rst",
  tex: "latex", ltx: "latex", sty: "latex", cls: "latex", bib: "bibtex",
  adoc: "asciidoc", asciidoc: "asciidoc",
  typ: "typst", wiki: "wikitext", po: "po", pot: "po",
  mermaid: "mermaid", mmd: "mermaid",
  diff: "diff", patch: "diff",
  http: "http", rest: "http", log: "log",
  feature: "gherkin", nginx: "nginx", awk: "awk", ipynb: "json",
};

// `#!/usr/bin/env python3` and friends. Last resort, for the extension-less
// scripts that live in every repo's scripts/ directory.
const BY_INTERPRETER: Record<string, string> = {
  sh: "shellscript", bash: "shellscript", zsh: "shellscript", ksh: "shellscript",
  dash: "shellscript", ash: "shellscript", fish: "fish",
  python: "python", python2: "python", python3: "python",
  node: "javascript", bun: "javascript", deno: "typescript",
  ruby: "ruby", perl: "perl", php: "php", lua: "lua", tclsh: "tcl",
  rscript: "r", julia: "julia", groovy: "groovy", pwsh: "powershell",
  awk: "awk", gawk: "awk", nu: "nushell",
};

/** Own-property lookup: `key in map` would match "constructor" and friends
 *  off Object.prototype and hand back a function instead of a language id. */
function lookup(map: Record<string, string>, key: string): string | null {
  return Object.prototype.hasOwnProperty.call(map, key) ? map[key] : null;
}

/** Interpreter name from a shebang line, e.g. "#!/usr/bin/env -S python3 -u" → "python3". */
function interpreterFromShebang(line: string): string | null {
  const m = /^#!\s*(\S+)((?:\s+\S+)*)/.exec(line);
  if (!m) return null;
  const bin = m[1].split("/").pop() ?? "";
  // `env` runs the real interpreter, which follows it (past any -S/-i flags
  // and VAR=value assignments).
  if (bin === "env") {
    for (const arg of m[2].trim().split(/\s+/)) {
      if (!arg || arg.startsWith("-") || arg.includes("=")) continue;
      return (arg.split("/").pop() ?? "").toLowerCase() || null;
    }
    return null;
  }
  return bin.toLowerCase() || null;
}

/**
 * Language id for a file path, or null when nothing sensible matches.
 * `firstLine` is optional; pass it to let shebangs identify scripts that have
 * no extension at all.
 */
export function languageIdForPath(path: string, firstLine?: string): string | null {
  const name = (path.split(/[\\/]/).pop() ?? "").toLowerCase();

  const named = lookup(BY_NAME, name);
  if (named) return named;
  // Dockerfile.web, .env.local, Makefile.inc, … keep the base file's language.
  const dotted = name.split(".");
  if (dotted.length > 1) {
    const base = dotted[0];
    if (base === "dockerfile" || base === "makefile" || base === "jenkinsfile") return BY_NAME[base];
    if (name === ".env" || name.startsWith(".env.")) return "dotenv";
  }

  // Try the two-part extension first (`.blade.php`, `.cmake.in`), then the
  // last segment, so a registered compound wins over its plain suffix.
  for (let i = Math.max(1, dotted.length - 2); i < dotted.length; i++) {
    const byExt = lookup(BY_EXT, dotted.slice(i).join("."));
    if (byExt) return byExt;
  }

  if (firstLine) {
    const interp = interpreterFromShebang(firstLine);
    if (interp) return lookup(BY_INTERPRETER, interp);
  }
  return null;
}

/** Every language id this map can produce. Used by the tests that keep the
 *  id space in step with Shiki's and CodeMirror's grammar registries. */
export function knownLanguageIds(): string[] {
  return [...new Set([...Object.values(BY_NAME), ...Object.values(BY_EXT), ...Object.values(BY_INTERPRETER)])].sort();
}
