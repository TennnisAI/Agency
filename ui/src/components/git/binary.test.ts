import { describe, expect, it } from "vitest";
import { emptyReason, formatSize, isRasterImage } from "./binary";

describe("isRasterImage", () => {
  it("claims the formats the picture view can render", () => {
    for (const p of ["a.png", "dir/b.JPG", "c.jpeg", "d.gif", "e.webp", "f.bmp", "g.ico", "h.avif"]) {
      expect(isRasterImage(p), p).toBe(true);
    }
  });

  it("leaves text, other binaries, and extensionless files to the line viewer", () => {
    // svg is an image but also text: its line diff is the more useful read.
    for (const p of ["logo.svg", "a.ts", "notes.md", "doc.pdf", "font.woff2", "Makefile"]) {
      expect(isRasterImage(p), p).toBe(false);
    }
  });
});

describe("emptyReason", () => {
  it("names a binary change rather than calling it no change", () => {
    const header = "diff --git a/i.png b/i.png\nindex e285ef8..212492e 100644\nBinary files a/i.png and b/i.png differ\n";
    expect(emptyReason(header)).toMatch(/^Binary file changed/);
  });

  it("recognizes a binary patch produced with --binary", () => {
    expect(emptyReason("diff --git a/i.png b/i.png\nGIT binary patch\n")).toMatch(/^Binary file changed/);
  });

  it("explains a mode-only change", () => {
    const header = "diff --git a/run.sh b/run.sh\nold mode 100644\nnew mode 100755\n";
    expect(emptyReason(header)).toMatch(/permissions/);
  });

  it("falls back to no textual changes", () => {
    expect(emptyReason("")).toBe("no textual changes");
  });
});

describe("formatSize", () => {
  it("scales the unit to the size", () => {
    expect(formatSize(0)).toBe("0 B");
    expect(formatSize(512)).toBe("512 B");
    expect(formatSize(2048)).toBe("2.0 KB");
    expect(formatSize(1024 * 1024 * 3)).toBe("3.0 MB");
  });
});
