import { describe, expect, it } from "vitest";
import {
  attachmentMarkdown,
  attachmentName,
  baseName,
  insertAttachment,
  isImageName,
  parseAttachments,
  pastedName,
  removeAttachment,
  slugify,
  splitExt,
  stampFor,
} from "./attachments";

describe("names", () => {
  it("splits extensions, leaving dotfiles alone", () => {
    expect(splitExt("shot.PNG")).toEqual({ base: "shot", ext: "png" });
    expect(splitExt("a.tar.gz")).toEqual({ base: "a.tar", ext: "gz" });
    expect(splitExt("README")).toEqual({ base: "README", ext: "" });
    expect(splitExt(".gitignore")).toEqual({ base: ".gitignore", ext: "" });
  });

  it("takes the last segment of either separator", () => {
    expect(baseName("<home>/Desktop/shot.png")).toBe("shot.png");
    expect(baseName("C:\\Users\\nic\\shot.png")).toBe("shot.png");
    expect(baseName("shot.png")).toBe("shot.png");
  });

  it("slugifies to a link-safe token", () => {
    expect(slugify("Screen Shot 2026-08-02 at 13.45.01")).toBe("screen-shot-2026-08-02-at-13-45-01");
    expect(slugify("  ¡Hola! ")).toBe("hola");
    // Empty when there's nothing to keep — callers pick the fallback. Never
    // left with a trailing dash after the length clamp.
    expect(slugify("***")).toBe("");
    expect(slugify("a".repeat(80)).length).toBeLessThanOrEqual(48);
    expect(slugify(`${"a".repeat(47)} b`)).not.toMatch(/-$/);
  });

  it("keeps a dropped file's name and stamps a pasted one", () => {
    const stamp = "20260802-134501";
    expect(attachmentName("AGE-13", "Login crash.png", stamp)).toBe("age-13-login-crash.png");
    expect(attachmentName("AGE-13", null, stamp)).toBe("age-13-20260802-134501.png");
    // Collision retries get a counter, not a new timestamp.
    expect(attachmentName("AGE-13", "shot.png", stamp, 3)).toBe("age-13-shot-3.png");
    // Extensionless files survive.
    expect(attachmentName("AGE-13", "Makefile", stamp)).toBe("age-13-makefile");
    // A name that slugifies away falls back to the stamp.
    expect(attachmentName("AGE-13", "***.png", stamp)).toBe(`age-13-${stamp}.png`);
  });

  it("names pasted blobs from their mime type", () => {
    const stamp = "20260802-134501";
    expect(pastedName("AGE-13", "image/jpeg", stamp)).toBe("age-13-20260802-134501.jpg");
    expect(pastedName("AGE-13", "image/svg+xml", stamp)).toBe("age-13-20260802-134501.svg");
    // Unknown types still land as something openable.
    expect(pastedName("AGE-13", "image/heic", stamp)).toBe("age-13-20260802-134501.png");
    expect(pastedName("AGE-13", "image/png", stamp, 2)).toBe("age-13-20260802-134501-2.png");
  });

  it("stamps local time zero-padded", () => {
    expect(stampFor(new Date(2026, 7, 2, 3, 4, 5))).toBe("20260802-030405");
  });

  it("recognizes renderable images by extension", () => {
    for (const n of ["a.png", "a.JPG", "a.jpeg", "a.gif", "a.webp", "a.svg", "a.avif"]) {
      expect(isImageName(n), n).toBe(true);
    }
    for (const n of ["a.pdf", "a.txt", "a.mov", "a.zip", "notes"]) {
      expect(isImageName(n), n).toBe(false);
    }
  });

  it("embeds images and links everything else", () => {
    expect(attachmentMarkdown("age-13-shot.png", "login crash")).toBe(
      "![login crash](assets/age-13-shot.png)",
    );
    expect(attachmentMarkdown("age-13-trace.txt")).toBe("[age-13-trace.txt](assets/age-13-trace.txt)");
  });
});

describe("parseAttachments", () => {
  it("finds embeds and links under assets/, in order", () => {
    const body = "Before\n\n![shot](assets/a.png)\n\nAnd [the log](assets/b.txt) after.\n";
    const found = parseAttachments(body);
    expect(found.map((a) => a.name)).toEqual(["a.png", "b.txt"]);
    expect(found[0]).toMatchObject({
      ref: "assets/a.png",
      repoPath: ".agency/issues/assets/a.png",
      alt: "shot",
      embedded: true,
      isImage: true,
    });
    expect(found[1]).toMatchObject({ embedded: false, isImage: false, alt: "the log" });
    expect(body.slice(found[0].from, found[0].to)).toBe("![shot](assets/a.png)");
  });

  it("ignores links that aren't attachments", () => {
    const body = [
      "[docs](https://example.com/a.png)",
      "![](../elsewhere/a.png)",
      "[up](assets/../../secret.png)",
      "[note](other/a.png)",
      "plain assets/a.png text",
    ].join("\n");
    expect(parseAttachments(body)).toEqual([]);
  });

  it("reads angle-bracket and percent-encoded targets other editors write", () => {
    const found = parseAttachments("![](<assets/a b.png>) and ![](assets/c%20d.png)");
    expect(found.map((a) => a.ref)).toEqual(["assets/a b.png", "assets/c d.png"]);
    // Undecodable escapes fall back to the raw text rather than throwing.
    expect(parseAttachments("![](assets/100%.png)")[0].ref).toBe("assets/100%.png");
  });

  it("collapses a file referenced twice to its first mention", () => {
    const found = parseAttachments("![](assets/a.png)\n\nsee ![](assets/a.png) again");
    expect(found).toHaveLength(1);
    expect(found[0].from).toBe(0);
  });
});

describe("insertAttachment", () => {
  it("splices at the caret and reports where it lands", () => {
    const r = insertAttachment("see ", 4, 4, "![](assets/a.png)");
    expect(r.body).toBe("see ![](assets/a.png)");
    expect(r.cursor).toBe(r.body.length);
  });

  it("replaces a selection", () => {
    expect(insertAttachment("see XXX now", 4, 7, "![](assets/a.png)").body).toBe(
      "see ![](assets/a.png) now",
    );
  });

  it("pads off a preceding word but not a newline or existing space", () => {
    expect(insertAttachment("word", 4, 4, "X").body).toBe("word X");
    expect(insertAttachment("line\n", 5, 5, "X").body).toBe("line\nX");
    expect(insertAttachment("", 0, 0, "X").body).toBe("X");
    expect(insertAttachment("word ", 5, 5, "X").body).toBe("word X");
  });
});

describe("removeAttachment", () => {
  const only = (body: string) => removeAttachment(body, parseAttachments(body)[0]);

  it("takes the whole line when the link was alone on it", () => {
    expect(only("Before\n\n![](assets/a.png)\n\nAfter\n")).toBe("Before\n\nAfter\n");
  });

  it("leaves surrounding text intact when the link is inline", () => {
    expect(only("see ![](assets/a.png) here")).toBe("see  here");
  });

  it("handles a link on the first and last line", () => {
    expect(only("![](assets/a.png)\nrest")).toBe("rest");
    expect(only("rest\n![](assets/a.png)")).toBe("rest\n");
    expect(only("![](assets/a.png)")).toBe("");
  });

  it("removes only the targeted attachment", () => {
    const body = "![](assets/a.png)\n![](assets/b.png)\n";
    const b = parseAttachments(body)[1];
    expect(removeAttachment(body, b)).toBe("![](assets/a.png)\n");
  });
});
