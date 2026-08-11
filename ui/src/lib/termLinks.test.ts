import { describe, it, expect } from "vitest";
import { Terminal } from "@xterm/xterm";
import { linkRange, logicalLine, scanLine, type TermLinkCandidate } from "./termLinks";

const targets = (line: string) => scanLine(line).map((c) => c.target);
const at = (line: string, target: string): TermLinkCandidate =>
  scanLine(line).find((c) => c.target === target)!;

describe("scanLine", () => {
  it("finds urls, whatever prose is around them", () => {
    expect(targets("Preview is at http://localhost:5173/ now")).toEqual(["http://localhost:5173/"]);
    expect(targets("See https://docs.rs/tauri, then decide.")).toEqual(["https://docs.rs/tauri"]);
    expect(targets("PR opened: <https://github.com/o/r/pull/3>")).toEqual([
      "https://github.com/o/r/pull/3",
    ]);
    expect(targets("write to mailto:nic@example.com")).toEqual(["mailto:nic@example.com"]);
    expect(targets("try www.example.com/docs")).toEqual(["https://www.example.com/docs"]);
  });

  it("keeps a bracket the url opened itself", () => {
    expect(targets("https://en.wikipedia.org/wiki/Terminal_(macOS)")).toEqual([
      "https://en.wikipedia.org/wiki/Terminal_(macOS)",
    ]);
    expect(targets("(https://example.com/a)")).toEqual(["https://example.com/a"]);
  });

  it("ignores a scheme with nothing behind it", () => {
    expect(targets("https:// is not a link")).toEqual([]);
  });

  it("finds paths in every shape a tool prints them", () => {
    expect(targets("edited src/lib/termLinks.ts today")).toEqual(["src/lib/termLinks.ts"]);
    expect(targets("see <home>/agency/README.md")).toEqual(["<home>/agency/README.md"]);
    expect(targets("run ./dev.sh and ../other/x.rs")).toEqual(["./dev.sh", "../other/x.rs"]);
    expect(targets("open ~/notes/today.md")).toEqual(["~/notes/today.md"]);
    expect(targets("bumped package.json and Cargo.toml")).toEqual(["package.json", "Cargo.toml"]);
    expect(targets("in src/components/")).toEqual(["src/components/"]);
  });

  it("carries the line number a compiler tacked on", () => {
    expect(at("error at src/main.rs:42:9: mismatched types", "src/main.rs")).toMatchObject({
      kind: "path",
      line: 42,
    });
    expect(at("src/main.rs:42 warning", "src/main.rs").line).toBe(42);
    expect(at("src/main.rs is fine", "src/main.rs").line).toBeUndefined();
  });

  it("strips the punctuation and chrome a line wraps a path in", () => {
    expect(targets("(src/main.rs)")).toEqual(["src/main.rs"]);
    expect(targets("│ src/main.rs │")).toEqual(["src/main.rs"]);
    expect(targets("wrote 'src/main.rs', then \"docs/a.md\".")).toEqual([
      "src/main.rs",
      "docs/a.md",
    ]);
    expect(targets("**src/main.rs**")).toEqual(["src/main.rs"]);
  });

  it("leaves ordinary prose alone", () => {
    expect(targets("bumped to v1.2.3 (was 1.2)")).toEqual([]);
    expect(targets("100% of 12 tests, 0.5s")).toEqual([]);
    expect(targets("mail nic@example.com about it")).toEqual([]);
    expect(targets("")).toEqual([]);
  });

  it("hands the last word-shaped cases to the disk rather than guessing", () => {
    // "e.g" is the shape of a filename and only the filesystem can say it
    // isn't one — no link is offered for a candidate that doesn't resolve.
    expect(targets("done, e.g. the tests pass")).toEqual(["e.g"]);
  });

  it("reports offsets that cover the link and nothing else", () => {
    const line = "see (src/main.rs:42) now";
    const [hit] = scanLine(line);
    expect(line.slice(hit.start, hit.end)).toBe("src/main.rs:42");
  });
});

describe("logicalLine", () => {
  // A pane that wraps: everything past `cols` continues on the next row, and a
  // path split across the edge has to come back as one string.
  async function pane(cols: number, text: string) {
    const term = new Terminal({ cols, rows: 6, scrollback: 100 });
    await new Promise<void>((done) => term.write(text, done));
    return term;
  }

  it("joins a wrapped line back together and says where it starts", async () => {
    const term = await pane(20, "open src/very/long/path/to/main.rs now");
    // The path straddles the wrap, so only the joined line can find it.
    const line = logicalLine(term.buffer.active, 2)!;
    expect(line.firstRow).toBe(0);
    expect(line.text).toBe("open src/very/long/path/to/main.rs now");
    expect(targets(line.text)).toEqual(["src/very/long/path/to/main.rs"]);
  });

  it("stops at the edges of the logical line", async () => {
    const term = await pane(20, "first line\r\nsecond one here\r\n");
    expect(logicalLine(term.buffer.active, 1)!.text).toBe("first line");
    expect(logicalLine(term.buffer.active, 2)!.text).toBe("second one here");
  });

  it("has nothing to say about a row off the end of the buffer", async () => {
    const term = await pane(20, "x");
    expect(logicalLine(term.buffer.active, 0)).toBeNull();
    expect(logicalLine(term.buffer.active, term.buffer.active.length + 1)).toBeNull();
  });
});

describe("linkRange", () => {
  it("maps an offset span onto 1-based cells, end inclusive", () => {
    expect(linkRange(20, 0, 5, 10)).toEqual({ start: { x: 6, y: 1 }, end: { x: 10, y: 1 } });
  });

  it("carries a span across the wrap onto the next row", () => {
    // cols 20: offsets 18..22 start two cells before the edge and end on row 2.
    expect(linkRange(20, 3, 18, 23)).toEqual({ start: { x: 19, y: 4 }, end: { x: 3, y: 5 } });
  });
});
