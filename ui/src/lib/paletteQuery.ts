// Palette query modes (one-stop Phase 4): the first character routes to a
// provider — ">" commands, "@" issues, "#" tags, "/" file contents — and
// everything else is the default project/run/recents mode.

export type PaletteMode = "default" | "command" | "issue" | "tag" | "content";

export interface PaletteQuery {
  mode: PaletteMode;
  /** Query with the mode prefix stripped, trimmed. */
  term: string;
}

const PREFIXES: Record<string, PaletteMode> = {
  ">": "command",
  "@": "issue",
  "#": "tag",
  "/": "content",
};

export function parsePaletteQuery(raw: string): PaletteQuery {
  const mode = PREFIXES[raw[0] ?? ""] ?? "default";
  const term = (mode === "default" ? raw : raw.slice(1)).trim();
  return { mode, term };
}
