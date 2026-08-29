import { describe, it, expect } from "vitest";
import { Terminal } from "@xterm/xterm";
import {
  createRecolor, indexedRgb, isDarkNeutral, parseHex, recolor, rewriteSgr,
} from "./termPaper";
import { resolveTheme, surfaceFor } from "./themes";

/** Paperback's `s1`, the surface a light theme recolors onto. */
const PAPER = surfaceFor(resolveTheme("paperback"))!;
const TO = parseHex(PAPER)!;
const ON_PAPER = "48;2;151;162;150";

const bytes = (s: string) => new TextEncoder().encode(s);
const text = (b: Uint8Array) => new TextDecoder().decode(b);

describe("rewriteSgr", () => {
  it("recolors the near-black bar an agent paints behind your prompt (AGE-165)", () => {
    // Claude Code's own `userMessageBackground`, rgb(55, 55, 55).
    expect(rewriteSgr("48;2;55;55;55", TO)).toBe(ON_PAPER);
    expect(rewriteSgr("0;48;2;55;55;55", TO)).toBe(`0;${ON_PAPER}`);
    // And its hover, rgb(70, 70, 70), and its composer, rgb(38, 38, 38).
    expect(rewriteSgr("48;2;70;70;70", TO)).toBe(ON_PAPER);
    expect(rewriteSgr("48;2;38;38;38", TO)).toBe(ON_PAPER);
  });

  it("recolors a dark background named by palette index", () => {
    expect(rewriteSgr("48;5;236", TO)).toBe(ON_PAPER); // the grayscale ramp
    expect(rewriteSgr("48;5;16", TO)).toBe(ON_PAPER); // black, in the cube
  });

  it("leaves the sixteen ANSI colors to the theme", () => {
    expect(rewriteSgr("40", TO)).toBeNull();
    expect(rewriteSgr("48;5;0", TO)).toBeNull();
    expect(rewriteSgr("48;5;8", TO)).toBeNull();
  });

  it("leaves a dark background that carries a color", () => {
    // Claude Code's selection, rgb(38, 79, 120). Flattening it onto one paper
    // tone would lose what it is saying; the contrast floor keeps its text
    // readable either way.
    expect(rewriteSgr("48;2;38;79;120", TO)).toBeNull();
    expect(rewriteSgr("48;5;52", TO)).toBeNull(); // dark red, in the cube
  });

  it("leaves a background that is already light", () => {
    expect(rewriteSgr("48;2;220;220;220", TO)).toBeNull();
    expect(rewriteSgr("48;5;254", TO)).toBeNull();
  });

  it("never touches a foreground", () => {
    expect(rewriteSgr("38;2;55;55;55", TO)).toBeNull();
    expect(rewriteSgr("38;5;236", TO)).toBeNull();
    expect(rewriteSgr("58;2;55;55;55", TO)).toBeNull();
  });

  it("steps over a foreground's parameters rather than reading them as its own", () => {
    // The `48` here is the fg's blue channel, not a background.
    expect(rewriteSgr("38;2;0;0;48", TO)).toBeNull();
    expect(rewriteSgr("1;38;2;0;0;48;48;2;55;55;55", TO)).toBe(`1;38;2;0;0;48;${ON_PAPER}`);
  });

  it("passes the ISO colon form through untouched", () => {
    // Terminals disagree about its shape; repainting the wrong parameter is
    // worse than leaving a dark bar alone.
    expect(rewriteSgr("48:2::55:55:55", TO)).toBeNull();
  });

  it("ignores a run that the parameters cut short, or that is not a number", () => {
    expect(rewriteSgr("48;2;55;55", TO)).toBeNull();
    expect(rewriteSgr("48;5", TO)).toBeNull();
    expect(rewriteSgr("48;2;55;55;x", TO)).toBeNull();
    expect(rewriteSgr("", TO)).toBeNull();
  });
});

describe("indexedRgb", () => {
  it("reproduces the xterm 256-color palette", () => {
    expect(indexedRgb(15)).toBeNull(); // the theme's own
    expect(indexedRgb(16)).toEqual([0, 0, 0]);
    expect(indexedRgb(196)).toEqual([255, 0, 0]);
    expect(indexedRgb(231)).toEqual([255, 255, 255]);
    expect(indexedRgb(232)).toEqual([8, 8, 8]);
    expect(indexedRgb(236)).toEqual([48, 48, 48]);
    expect(indexedRgb(255)).toEqual([238, 238, 238]);
    expect(indexedRgb(256)).toBeNull();
  });
});

describe("isDarkNeutral", () => {
  it("takes grays, not colors, and only the dark ones", () => {
    expect(isDarkNeutral([55, 55, 55])).toBe(true);
    expect(isDarkNeutral([40, 44, 58])).toBe(true); // catppuccin's own surface
    expect(isDarkNeutral([38, 79, 120])).toBe(false); // blue
    expect(isDarkNeutral([200, 200, 200])).toBe(false); // light
  });
});

describe("parseHex", () => {
  it("takes #rrggbb and nothing else", () => {
    expect(parseHex("#a7b1a6")).toEqual([167, 177, 166]);
    expect(parseHex("#A7B1A6")).toEqual([167, 177, 166]);
    expect(parseHex("a7b1a6")).toBeNull();
    expect(parseHex("var(--s0)")).toBeNull();
  });
});

describe("recolor", () => {
  it("rewrites the sequences and leaves everything else byte for byte", () => {
    const out = text(recolor(bytes("a\x1b[0;48;2;55;55;55m ❯ prompt \x1b[0m\r\nb"), PAPER));
    expect(out).toBe(`a\x1b[0;${ON_PAPER}m ❯ prompt \x1b[0m\r\nb`);
  });

  it("is a no-op on a dark theme, down to the same array", () => {
    const src = bytes("\x1b[48;2;55;55;55m");
    expect(recolor(src, null)).toBe(src);
    expect(recolor(src, "not a color")).toBe(src);
  });

  it("leaves a stream with nothing to change alone, down to the same array", () => {
    const src = bytes("\x1b[1;31mplain\x1b[0m");
    expect(recolor(src, PAPER)).toBe(src);
  });

  it("keeps its hands off every other sequence", () => {
    const src = "\x1b[?1049h\x1b]0;a title\x07\x1b]8;;https://x/y\x1b\\link\x1b[2J\x1b[<35;1;1M";
    expect(text(recolor(bytes(src), PAPER))).toBe(src);
  });

  it("passes a sequence it cannot finish through as it stands", () => {
    const src = "text\x1b[48;2;55;55";
    expect(text(recolor(bytes(src), PAPER))).toBe(src);
  });

  it("resyncs on a malformed sequence instead of swallowing what follows", () => {
    const src = "\x1b[\x00m\x1b[48;2;55;55;55m";
    expect(text(recolor(bytes(src), PAPER))).toBe(`\x1b[\x00m\x1b[${ON_PAPER}m`);
  });
});

describe("createRecolor", () => {
  const paper = () => PAPER as string | null;

  it("holds a sequence split across chunks until the chunk that finishes it", () => {
    const filter = createRecolor(paper);
    expect(text(filter(bytes("row\x1b[48;2;55")))).toBe("row");
    expect(text(filter(bytes(";55;55m ❯ ")))).toBe(`\x1b[${ON_PAPER}m ❯ `);
  });

  it("holds a bare escape at the end of a chunk", () => {
    const filter = createRecolor(paper);
    expect(text(filter(bytes("row\x1b")))).toBe("row");
    expect(text(filter(bytes("[48;5;236mx")))).toBe(`\x1b[${ON_PAPER}mx`);
  });

  it("gives up on a sequence that runs past any real one, rather than stalling", () => {
    const filter = createRecolor(paper);
    const long = "\x1b[" + "1;".repeat(200);
    expect(text(filter(bytes(long)))).toBe(long);
  });

  it("hands back what it is holding when the theme goes dark mid-sequence", () => {
    let surface: string | null = PAPER;
    const filter = createRecolor(() => surface);
    expect(text(filter(bytes("row\x1b[48;2;55")))).toBe("row");
    surface = null;
    expect(text(filter(bytes(";55;55m")))).toBe("\x1b[48;2;55;55;55m");
  });
});

// The point of all of the above: what the pane actually paints. Headless xterm,
// parsing the stream the way FocusTerminal hands it over.
describe("against a live xterm", () => {
  const cellBg = async (data: Uint8Array) => {
    const term = new Terminal({ rows: 4, cols: 20 });
    await new Promise<void>((done) => term.write(data, done));
    const cell = term.buffer.active.getLine(0)!.getCell(0)!;
    return { rgb: cell.isBgRGB(), color: cell.getBgColor() };
  };

  it("paints the prompt bar on paper instead of near-black", async () => {
    const bar = bytes("\x1b[48;2;55;55;55m ❯ prompt");
    expect(await cellBg(bar)).toEqual({ rgb: true, color: 0x373737 });
    expect(await cellBg(recolor(bar, PAPER))).toEqual({ rgb: true, color: 0x97a296 });
  });

  it("paints an indexed dark background on paper too", async () => {
    const bar = bytes("\x1b[48;5;236m code ");
    expect(await cellBg(bar)).toEqual({ rgb: false, color: 236 });
    expect(await cellBg(recolor(bar, PAPER))).toEqual({ rgb: true, color: 0x97a296 });
  });

  it("leaves a colored background where the agent put it", async () => {
    const sel = bytes("\x1b[48;2;38;79;120mselected");
    expect(await cellBg(recolor(sel, PAPER))).toEqual({ rgb: true, color: 0x264f78 });
  });
});
