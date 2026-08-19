import { describe, expect, it } from "vitest";
import { attachmentLink, relativeFrom } from "./noteLink";

describe("relativeFrom", () => {
  it("is the bare name for a file beside the note", () => {
    expect(relativeFrom("guides/setup.md", "guides/shot.png")).toBe("shot.png");
    expect(relativeFrom("home.md", "shot.png")).toBe("shot.png");
  });

  it("descends into a subfolder without a prefix", () => {
    expect(relativeFrom("home.md", "assets/shot.png")).toBe("assets/shot.png");
    expect(relativeFrom("guides/setup.md", "guides/img/shot.png")).toBe("img/shot.png");
  });

  it("climbs out of the note's folder, one `..` per level", () => {
    expect(relativeFrom("guides/setup.md", "assets/shot.png")).toBe("../assets/shot.png");
    expect(relativeFrom("a/b/c/deep.md", "assets/shot.png")).toBe("../../../assets/shot.png");
  });

  it("keeps the shared prefix and climbs only the rest", () => {
    expect(relativeFrom("a/b/note.md", "a/c/shot.png")).toBe("../c/shot.png");
  });

  it("does not treat a folder name matching the file name as shared", () => {
    // The last target segment is the file, so it can never pair with a folder.
    expect(relativeFrom("assets/note.md", "assets")).toBe("../assets");
  });
});

describe("attachmentLink", () => {
  it("embeds images and links everything else by name", () => {
    expect(attachmentLink("home.md", "assets/shot.png")).toBe("![](assets/shot.png)");
    expect(attachmentLink("home.md", "assets/spec.pdf")).toBe("[spec.pdf](assets/spec.pdf)");
  });

  it("wraps a path markdown cannot hold raw in angle brackets", () => {
    expect(attachmentLink("home.md", "assets/my shot.png")).toBe("![](<assets/my shot.png>)");
    expect(attachmentLink("home.md", "notes (old).txt")).toBe(
      "[notes (old).txt](<notes (old).txt>)",
    );
  });

  it("writes the note-relative form from a subfolder", () => {
    expect(attachmentLink("guides/setup.md", "assets/shot.png")).toBe("![](../assets/shot.png)");
  });
});
