/**
 * Dark backgrounds an agent paints, put back onto the theme's paper.
 *
 * AGE-165: on Paperback, the bar behind your own prompt in a Claude Code pane
 * was near-black. An agent draws with the palette *its* theme picked, and a
 * dark-theme agent in a light pane paints `\x1b[48;2;55;55;55m` — Claude Code's
 * `userMessageBackground`, rgb(55, 55, 55) — straight onto paper.
 *
 * Nothing in xterm can catch that. `minimumContrastRatio` (see themes.ts) lifts
 * a *foreground* against the background it sits on, which is why dark-theme text
 * stays readable here; no option touches a background the app set itself. And
 * `ITheme` only names the 256 palette entries, so a truecolor background is out
 * of its reach even in principle. The one place left is the byte stream, before
 * xterm parses it.
 *
 * So: an SGR that sets a dark, near-neutral background gets the light theme's
 * own surface instead (`surfaceFor`, themes.ts). Two deliberate limits:
 *
 *  - Backgrounds only (`48`), never foregrounds (`38`/`58`). A dark foreground
 *    on paper is already right, and where it isn't, the contrast floor has it.
 *  - Near-neutral only. A colored dark background carries meaning — Claude
 *    Code's selection is rgb(38, 79, 120) — and flattening every one of them
 *    onto a single paper tone would lose it. Text on those stays readable by
 *    the contrast floor, same as anywhere else.
 *
 * The stream form holds back a trailing partial escape, because a chunk really
 * can end mid-sequence: `term/pty.rs` stitches the pieces the kernel tears a
 * write into, but its ceilings (4ms, 128 kB) cut wherever they land.
 *
 * This runs on the way in, so cells keep whatever they were parsed with: switch
 * to a light theme with a pane already open and its old bars stay dark until
 * the agent repaints them or the pane is opened again. The contrast floor, by
 * contrast, is applied at render time and changes with the theme at once.
 */

const ESC = 0x1b;
const CSI_OPEN = 0x5b; // '['
const SGR_FINAL = 0x6d; // 'm'

/** Channels this close together read as a gray, not as a color. */
const NEUTRAL_SPREAD = 24;
/** A mean channel below this is a surface a dark theme drew to sit under text. */
const DARK_MEAN = 128;

const encoder = new TextEncoder();

export type Rgb = [number, number, number];

/** The 24-step ramp the xterm 256-color palette ends with. */
const GRAY_BASE = 8;
const GRAY_STEP = 10;
/** The six levels the 6x6x6 color cube is built from. */
const CUBE_LEVELS = [0, 95, 135, 175, 215, 255];

/** Parse `#rrggbb`. Returns null for anything else, so a bad theme value is a
 *  no-op rather than a pane full of rewritten garbage. */
export function parseHex(hex: string): Rgb | null {
  const m = /^#([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const n = parseInt(m[1], 16);
  return [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
}

/** What xterm renders for a 256-palette index. Indices 0-15 are the theme's own
 *  (themes.ts names all sixteen), so they are left to it. */
export function indexedRgb(i: number): Rgb | null {
  if (i < 16 || i > 255) return null;
  if (i >= 232) {
    const v = GRAY_BASE + (i - 232) * GRAY_STEP;
    return [v, v, v];
  }
  const n = i - 16;
  return [CUBE_LEVELS[Math.floor(n / 36)], CUBE_LEVELS[Math.floor(n / 6) % 6], CUBE_LEVELS[n % 6]];
}

export function isDarkNeutral([r, g, b]: Rgb): boolean {
  const spread = Math.max(r, g, b) - Math.min(r, g, b);
  return spread <= NEUTRAL_SPREAD && (r + g + b) / 3 < DARK_MEAN;
}

function param(tok: string): number | null {
  if (tok === "") return 0; // an omitted parameter is its default
  if (!/^\d+$/.test(tok)) return null;
  return parseInt(tok, 10);
}

/** Rewrite one SGR parameter string, or return null if it needs no change.
 *
 *  Only the `;`-separated forms are read — `48;5;n` and `48;2;r;g;b`, which is
 *  what every agent we run and our own snapshot writer emit. The ISO 8613-6
 *  `:`-separated form parses as one token here and so passes through untouched;
 *  terminals disagree about its shape, and guessing wrong would repaint the
 *  wrong parameter. */
export function rewriteSgr(params: string, to: Rgb): string | null {
  const tok = params.split(";");
  const out: string[] = [];
  let changed = false;
  let i = 0;
  while (i < tok.length) {
    // 38/48/58 introduce a color spelled out over the parameters that follow;
    // stepping over the whole run is what keeps an `r` of 48 from being read as
    // a background of its own.
    const kind = tok[i] === "38" || tok[i] === "48" || tok[i] === "58" ? param(tok[i + 1]) : null;
    const span = kind === 5 ? 3 : kind === 2 ? 5 : 0;
    if (span === 0 || i + span > tok.length) {
      out.push(tok[i]);
      i += 1;
      continue;
    }
    const parts = tok.slice(i, i + span).map(param);
    const rgb: Rgb | null =
      parts.some((p) => p === null)
        ? null
        : kind === 5
          ? indexedRgb(parts[2]!)
          : [parts[2]!, parts[3]!, parts[4]!];
    if (tok[i] === "48" && rgb && isDarkNeutral(rgb)) {
      out.push("48", "2", String(to[0]), String(to[1]), String(to[2]));
      changed = true;
    } else {
      out.push(...tok.slice(i, i + span));
    }
    i += span;
  }
  return changed ? out.join(";") : null;
}

const isParam = (b: number) => b >= 0x30 && b <= 0x3f;
const isIntermediate = (b: number) => b >= 0x20 && b <= 0x2f;
const isFinal = (b: number) => b >= 0x40 && b <= 0x7e;
/** A private-parameter prefix (`<=>?`) means this is not a plain SGR. */
const isPrefix = (b: number) => b >= 0x3c && b <= 0x3f;

function ascii(bytes: Uint8Array, from: number, to: number): string {
  let s = "";
  for (let i = from; i < to; i++) s += String.fromCharCode(bytes[i]);
  return s;
}

function join(parts: Uint8Array[]): Uint8Array {
  let total = 0;
  for (const p of parts) total += p.length;
  const out = new Uint8Array(total);
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

/** `out` is `bytes` with its dark backgrounds recolored, minus a trailing
 *  `held` bytes that begin a sequence this chunk does not finish. */
function scan(bytes: Uint8Array, to: Rgb): { out: Uint8Array; held: number } {
  let parts: Uint8Array[] | null = null;
  let copied = 0;
  let held = 0;
  let i = 0;
  while (i < bytes.length) {
    if (bytes[i] !== ESC) {
      i += 1;
      continue;
    }
    const start = i;
    if (start + 1 >= bytes.length) {
      held = bytes.length - start;
      break;
    }
    if (bytes[start + 1] !== CSI_OPEN) {
      i = start + 2; // some other escape; its own two bytes are not an SGR
      continue;
    }
    let at = start + 2;
    while (at < bytes.length && isParam(bytes[at])) at += 1;
    const paramsEnd = at;
    while (at < bytes.length && isIntermediate(bytes[at])) at += 1;
    if (at >= bytes.length) {
      held = bytes.length - start;
      break;
    }
    if (!isFinal(bytes[at])) {
      i = start + 1; // malformed — resync rather than swallow what follows
      continue;
    }
    const plain = at === paramsEnd && !isPrefix(bytes[start + 2]);
    if (bytes[at] === SGR_FINAL && plain) {
      const next = rewriteSgr(ascii(bytes, start + 2, paramsEnd), to);
      if (next !== null) {
        parts ??= [];
        parts.push(bytes.subarray(copied, start + 2), encoder.encode(next));
        copied = paramsEnd;
      }
    }
    i = at + 1;
  }
  const end = bytes.length - held;
  if (!parts) return { out: held ? bytes.subarray(0, end) : bytes, held };
  parts.push(bytes.subarray(copied, end));
  return { out: join(parts), held };
}

/** Recolor one self-contained chunk. A sequence left unfinished at its end is
 *  passed through as it stands; there is no later chunk to complete it. */
export function recolor(bytes: Uint8Array, paper: string | null): Uint8Array {
  const to = paper ? parseHex(paper) : null;
  if (!to) return bytes;
  const { out, held } = scan(bytes, to);
  return held ? join([out, bytes.subarray(bytes.length - held)]) : out;
}

/** Longest tail worth holding for the chunk that completes it. An SGR runs to a
 *  few dozen bytes at most, so anything longer is a sequence that will never be
 *  finished, and holding it would stall the pane on a stream that is already
 *  broken. */
const MAX_HELD = 128;

/** Recolor a continuous stream, holding a trailing partial sequence back until
 *  the chunk that finishes it. */
export function createRecolor(paper: () => string | null): (bytes: Uint8Array) => Uint8Array {
  let carry: Uint8Array | null = null;
  return (bytes) => {
    const whole = carry ? join([carry, bytes]) : bytes;
    carry = null;
    // A dark theme, or one switched to mid-sequence: hand back what is held so
    // the stream stays whole, and stop looking until a light theme comes round.
    const to = parseHex(paper() ?? "");
    if (!to) return whole;
    const { out, held } = scan(whole, to);
    if (held > MAX_HELD) return join([out, whole.subarray(whole.length - held)]);
    if (held) carry = whole.subarray(whole.length - held);
    return out;
  };
}
