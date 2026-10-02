import { describe, expect, it } from "vitest";
import { dropName, isMarkdown, leftOutGitNote, nameList, splitExt, uniqueName } from "./fileDrop";

describe("dropName", () => {
  it("takes the last segment, either separator", () => {
    expect(dropName("<home>/Desktop/notes.md")).toBe("notes.md");
    expect(dropName("C:\\Users\\nic\\notes.md")).toBe("notes.md");
    expect(dropName("notes.md")).toBe("notes.md");
  });
});

describe("splitExt", () => {
  it("splits on the last dot", () => {
    expect(splitExt("notes.md")).toEqual(["notes", ".md"]);
    expect(splitExt("archive.tar.gz")).toEqual(["archive.tar", ".gz"]);
  });

  it("leaves extensionless and dotfile names whole", () => {
    expect(splitExt("README")).toEqual(["README", ""]);
    expect(splitExt(".gitignore")).toEqual([".gitignore", ""]);
  });
});

describe("isMarkdown", () => {
  it("accepts both extensions, any case", () => {
    expect(isMarkdown("notes.md")).toBe(true);
    expect(isMarkdown("Notes.MARKDOWN")).toBe(true);
  });

  it("rejects everything else", () => {
    expect(isMarkdown("shot.png")).toBe(false);
    expect(isMarkdown("md")).toBe(false);
    expect(isMarkdown("notes.md.bak")).toBe(false);
  });
});

describe("uniqueName", () => {
  const taken = (...names: string[]) => new Set(names.map((n) => n.toLowerCase()));

  it("keeps a free name", () => {
    expect(uniqueName(taken("other.md"), "notes.md")).toBe("notes.md");
  });

  it("numbers before the extension, and keeps counting", () => {
    expect(uniqueName(taken("notes.md"), "notes.md")).toBe("notes 2.md");
    expect(uniqueName(taken("notes.md", "notes 2.md"), "notes.md")).toBe("notes 3.md");
  });

  it("matches case-insensitively, like the filesystem underneath", () => {
    expect(uniqueName(taken("NOTES.md"), "notes.md")).toBe("notes 2.md");
  });

  it("appends to extensionless names", () => {
    expect(uniqueName(taken("README"), "README")).toBe("README 2");
  });

  it("numbers a folder at the end, dots and all", () => {
    expect(uniqueName(taken("v1.2"), "v1.2", true)).toBe("v1.2 2");
    expect(uniqueName(taken("v1.2", "v1.2 2"), "v1.2", true)).toBe("v1.2 3");
    // Without the flag it is read as a file with an extension.
    expect(uniqueName(taken("v1.2"), "v1.2")).toBe("v1 2.2");
  });
});

describe("nameList", () => {
  it("names a short list in full", () => {
    expect(nameList(["a.md", "b.md"])).toBe("a.md, b.md");
  });

  it("counts the tail of a long one", () => {
    expect(nameList(["a", "b", "c", "d", "e"])).toBe("a, b, c and 2 more");
  });
});

describe("leftOutGitNote", () => {
  it("names the folders and says why", () => {
    expect(leftOutGitNote(["app", "lib"])).toBe(
      "Left out the .git in app, lib. Copied in, it would make a repository inside this one.",
    );
  });
});
