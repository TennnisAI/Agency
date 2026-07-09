import { describe, it, expect } from "vitest";
import { fileIcon } from "./fileIcon";

describe("fileIcon", () => {
  it("maps known extensions to a kind and a theme color var", () => {
    expect(fileIcon("main.rs")).toEqual({ kind: "code", color: "var(--red)" });
    expect(fileIcon("App.tsx")).toEqual({ kind: "code", color: "var(--blue)" });
    expect(fileIcon("data.json")).toEqual({ kind: "braces", color: "var(--peach)" });
    expect(fileIcon("styles.css").kind).toBe("style");
    expect(fileIcon("README.md")).toEqual({ kind: "doc", color: "var(--green)" });
    expect(fileIcon("logo.png").kind).toBe("image");
  });

  it("always returns a var(--…) color so icons stay in-palette", () => {
    for (const n of ["a.ts", "b.unknownext", "Makefile", ".env"]) {
      expect(fileIcon(n).color).toMatch(/^var\(--[a-z0-9]+\)$/);
    }
  });

  it("treats .env and its variants as secrets (lock)", () => {
    expect(fileIcon(".env").kind).toBe("lock");
    expect(fileIcon(".env.local").kind).toBe("lock");
    expect(fileIcon(".env.production").kind).toBe("lock");
  });

  it("matches whole names before extensions", () => {
    expect(fileIcon("package.json").color).toBe("var(--red)");
    expect(fileIcon("pnpm-lock.yaml").kind).toBe("lock");
    expect(fileIcon("Dockerfile").kind).toBe("config");
  });

  it("is case-insensitive", () => {
    expect(fileIcon("MAIN.RS").kind).toBe("code");
    expect(fileIcon("Package.JSON").color).toBe("var(--red)");
  });

  it("maps media, archives, fonts, and opaque binaries to their own kinds", () => {
    expect(fileIcon("song.mp3").kind).toBe("audio");
    expect(fileIcon("clip.mp4").kind).toBe("video");
    expect(fileIcon("scan.pdf").kind).toBe("pdf");
    expect(fileIcon("bundle.zip").kind).toBe("archive");
    expect(fileIcon("Inter.woff2").kind).toBe("font");
    expect(fileIcon("data.sqlite").kind).toBe("binary");
    // bmp/avif preview as images, so their tree icon should match.
    expect(fileIcon("photo.bmp").kind).toBe("image");
    expect(fileIcon("photo.avif").kind).toBe("image");
  });

  it("falls back to a generic file for unknown types", () => {
    expect(fileIcon("mystery")).toEqual({ kind: "file", color: "var(--sub1)" });
    expect(fileIcon("archive.xyz")).toEqual({ kind: "file", color: "var(--sub1)" });
  });
});
