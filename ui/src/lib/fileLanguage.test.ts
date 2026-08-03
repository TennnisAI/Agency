import { describe, expect, it } from "vitest";
import { bundledLanguages } from "shiki";
import { LanguageDescription } from "@codemirror/language";
import { knownLanguageIds, languageIdForPath } from "./fileLanguage";
import { CM_LANGUAGE_NAMES, CM_REGISTRY, languageDescription, languageForFence, loadLanguage } from "./cmLanguage";

describe("languageIdForPath", () => {
  it("maps common source extensions", () => {
    expect(languageIdForPath("src/app.tsx")).toBe("tsx");
    expect(languageIdForPath("src/lib/util.ts")).toBe("typescript");
    expect(languageIdForPath("src/lib/util.d.ts")).toBe("typescript");
    expect(languageIdForPath("main.go")).toBe("go");
    expect(languageIdForPath("crates/core/src/lib.rs")).toBe("rust");
    expect(languageIdForPath("a/b/query.sql")).toBe("sql");
    expect(languageIdForPath("deploy.yml")).toBe("yaml");
    expect(languageIdForPath("Cargo.toml")).toBe("toml");
    expect(languageIdForPath("script.sh")).toBe("shellscript");
    expect(languageIdForPath("main.kt")).toBe("kotlin");
    expect(languageIdForPath("Program.cs")).toBe("csharp");
    expect(languageIdForPath("infra/main.tf")).toBe("terraform");
    expect(languageIdForPath("schema.prisma")).toBe("prisma");
  });

  it("is case-insensitive and path-shape agnostic", () => {
    expect(languageIdForPath("SRC/App.TSX")).toBe("tsx");
    expect(languageIdForPath("C:\\work\\main.go")).toBe("go");
    expect(languageIdForPath("main.go")).toBe(languageIdForPath("./deep/nest/main.go"));
  });

  it("matches whole filenames that carry no extension", () => {
    expect(languageIdForPath("Dockerfile")).toBe("docker");
    expect(languageIdForPath("ops/Dockerfile.web")).toBe("docker");
    expect(languageIdForPath("Makefile")).toBe("make");
    expect(languageIdForPath("CMakeLists.txt")).toBe("cmake");
    expect(languageIdForPath("Gemfile")).toBe("ruby");
    expect(languageIdForPath("Jenkinsfile")).toBe("groovy");
    expect(languageIdForPath(".zshrc")).toBe("shellscript");
    expect(languageIdForPath(".gitignore")).toBe("ini");
    expect(languageIdForPath(".env")).toBe("dotenv");
    expect(languageIdForPath(".env.production")).toBe("dotenv");
  });

  it("prefers a registered extension over a dotted filename prefix", () => {
    expect(languageIdForPath(".eslintrc.json")).toBe("json");
    expect(languageIdForPath(".prettierrc.yaml")).toBe("yaml");
    expect(languageIdForPath(".eslintrc")).toBe("jsonc");
  });

  it("prefers a two-part extension over its plain suffix", () => {
    expect(languageIdForPath("views/home.blade.php")).toBe("blade");
    expect(languageIdForPath("views/home.php")).toBe("php");
    expect(languageIdForPath("config.cmake.in")).toBe("cmake");
  });

  it("falls back to the shebang for extension-less scripts", () => {
    expect(languageIdForPath("scripts/release", "#!/bin/bash")).toBe("shellscript");
    expect(languageIdForPath("scripts/release", "#!/usr/bin/env python3")).toBe("python");
    expect(languageIdForPath("scripts/release", "#!/usr/bin/env -S deno run")).toBe("typescript");
    expect(languageIdForPath("scripts/release", "#!/usr/bin/env NODE_ENV=dev node")).toBe("javascript");
    expect(languageIdForPath("scripts/release", "#!/usr/bin/perl -w")).toBe("perl");
  });

  it("lets the extension win over the shebang", () => {
    expect(languageIdForPath("build.py", "#!/bin/bash")).toBe("python");
  });

  it("returns null when nothing matches", () => {
    expect(languageIdForPath("notes.txt")).toBeNull();
    expect(languageIdForPath("LICENSE")).toBeNull();
    expect(languageIdForPath("data.unknownext")).toBeNull();
    expect(languageIdForPath("scripts/release", "not a shebang")).toBeNull();
  });

  // Filenames are attacker-shaped input in the sense that a repo can contain
  // anything; inherited Object keys must not read as languages.
  it("ignores names that collide with Object.prototype", () => {
    for (const name of ["constructor", "toString", "__proto__", "a.constructor", "a.__proto__"]) {
      expect(languageIdForPath(name), name).toBeNull();
    }
    expect(languageIdForPath("script", "#!/usr/bin/env constructor")).toBeNull();
  });
});

describe("language id coverage", () => {
  it("covers many more file types than the old seven-language list", () => {
    expect(knownLanguageIds().length).toBeGreaterThan(100);
  });

  // The ids are VS Code / TextMate ids so Shiki (which ships VS Code's own
  // grammars) can consume them directly. A typo here would silently downgrade
  // a diff to plain text.
  it("only produces ids Shiki knows", () => {
    const unknown = knownLanguageIds().filter((id) => !(id in bundledLanguages));
    expect(unknown).toEqual([]);
  });

  it("resolves the languages the editor claims to support", () => {
    const cases: Array<[string, string]> = [
      ["main.go", "Go"],
      ["app.tsx", "TSX"],
      ["deploy.yaml", "YAML"],
      ["Cargo.toml", "TOML"],
      ["run.sh", "Shell"],
      ["Dockerfile", "Dockerfile"],
      ["Makefile", "Makefile"],
      ["query.sql", "SQL"],
      ["main.c", "C"],
      ["View.swift", "Swift"],
      ["a.kt", "Kotlin"],
      ["a.rb", "Ruby"],
      ["a.php", "PHP"],
      ["a.cs", "C#"],
      ["a.lua", "Lua"],
      ["a.ex", "Elixir"],
      ["a.zig", "Zig"],
      ["main.tf", "Terraform"],
      ["schema.graphql", "GraphQL"],
      ["schema.prisma", "Prisma"],
      ["flake.nix", "Nix"],
      ["Token.sol", "Solidity"],
      ["blur.glsl", "Shader"],
      ["App.svelte", "HTML"],
      ["setup.cfg", "Properties files"],
      [".env", "Properties files"],
      ["notes.mdx", "Markdown"],
      ["mod.wat", "WebAssembly"],
      ["a.proto", "ProtoBuf"],
      ["a.f90", "Fortran"],
      ["a.mm", "Objective-C++"],
      ["README.rst", "reStructuredText"],
      ["rows.csv", "CSV"],
      ["build.bat", "Batch"],
      [".vimrc", "Vim script"],
      ["flow.mmd", "Mermaid"],
      ["main.nim", "Nim"],
      ["app.marko", "HTML"],
      ["COMMIT_EDITMSG", "Git commit"],
      ["server.log", "Log"],
    ];
    for (const [path, name] of cases) {
      expect(languageDescription(path)?.name, path).toBe(name);
    }
  });

  it("leaves unmatched files without a grammar", () => {
    expect(languageDescription("notes.txt")).toBeNull();
  });

  it("resolves markdown fence tags for docs notes", () => {
    const cases: Array<[string, string | null]> = [
      ["go", "Go"],
      ["py", "Python"],
      ["python", "Python"],
      ["sh", "Shell"],
      ["bash", "Shell"],
      ["shell", "Shell"],
      ["ts", "TypeScript"],
      ["typescript", "TypeScript"],
      ["rs", "Rust"],
      ["sql", "SQL"],
      ["graphql", "GraphQL"],
      ["json ansi", "JSON"], // fence tags can carry trailing metadata
      ["text", null],
      ["console", null],
      ["", null],
    ];
    for (const [info, name] of cases) {
      expect(languageForFence(info)?.name ?? null, info).toBe(name);
    }
  });

  // Grammars arrive as dynamic chunks, so a stale package name would only
  // surface when a user opens that file type. Load a spread of them for real.
  it("actually loads the grammar chunks", async () => {
    const paths = [
      "main.go", "app.tsx", "deploy.yaml", "Cargo.toml", "run.sh", "Dockerfile",
      "Makefile", "query.sql", "main.c", "a.rb", "main.tf", "blur.glsl", "a.zig",
    ];
    for (const path of paths) {
      const [support] = await loadLanguage(path);
      expect(support, path).toBeTruthy();
    }
  });

  it("resolves to no extension for a file it cannot place", async () => {
    expect(await loadLanguage("notes.txt")).toEqual([]);
  });

  // The two views used to disagree: Shiki ships VS Code's grammars for every
  // id, while CodeMirror covered about 150 of them, so the tail (rst, csv,
  // viml, mermaid, …) opened as flat text in the editor even though the diff
  // colored it. lib/cmModes.ts closes that gap, and this keeps it closed: a
  // new id needs a CodeMirror grammar, a redirect, or neither view gets it.
  it("gives the editor a grammar for every id the diff view colors", () => {
    const uncovered = knownLanguageIds().filter(
      (id) => !LanguageDescription.matchLanguageName(CM_REGISTRY, CM_LANGUAGE_NAMES[id] ?? id, false),
    );
    expect(uncovered).toEqual([]);
  });

  // A redirect that points at a grammar name which does not exist is always a
  // bug, even for an id nothing currently produces.
  it("has no redirect pointing at a missing grammar", () => {
    const broken = Object.entries(CM_LANGUAGE_NAMES)
      .filter(([, name]) => !LanguageDescription.matchLanguageName(CM_REGISTRY, name, false))
      .map(([id]) => id);
    expect(broken).toEqual([]);
  });
});
