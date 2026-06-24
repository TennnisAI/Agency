import { createHighlighter, type Highlighter, type ThemeRegistrationRaw } from "shiki";
import { currentThemeId, THEMES, type Theme } from "../../lib/themes";

const LANGS = ["typescript", "tsx", "javascript", "jsx", "json", "rust", "css", "html", "markdown", "python", "bash", "toml", "yaml"];
const EXT: Record<string, string> = {
  ts: "typescript", tsx: "tsx", js: "javascript", jsx: "jsx", json: "json",
  rs: "rust", css: "css", html: "html", md: "markdown", py: "python",
  sh: "bash", toml: "toml", yml: "yaml", yaml: "yaml",
};

export function langForPath(path: string): string {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  return EXT[ext] ?? "text";
}

// A Shiki TextMate theme built from the app theme's own palette. The bundled
// Shiki themes (e.g. catppuccin-latte) tune their muted colors — comments above
// all — for a near-white background, so they wash out on light themes whose
// background is a medium tone (Paperback's is a grey-green). Deriving the
// highlight from each theme's vars keeps every token, comment included, drawn
// against the same background it was picked for, so contrast holds.
function shikiThemeFor(theme: Theme): ThemeRegistrationRaw {
  const v = theme.vars;
  const scope = (scopes: string | string[], color: string, fontStyle?: string) => ({
    scope: scopes,
    settings: fontStyle ? { foreground: color, fontStyle } : { foreground: color },
  });
  return {
    name: `agency-${theme.id}`,
    type: theme.scheme,
    fg: v.text,
    bg: v.base,
    colors: { "editor.foreground": v.text, "editor.background": v.base },
    settings: [
      { settings: { foreground: v.text, background: v.base } },
      // `o1` is a muted-but-readable tone in every theme (e.g. Paperback #5a4f46),
      // unlike the near-invisible greys the stock light themes use for comments.
      scope(["comment", "punctuation.definition.comment"], v.o1, "italic"),
      scope(["string", "string.quoted", "constant.other.symbol", "meta.embedded.assembly"], v.green),
      scope(["constant.character.escape", "string.regexp"], v.teal),
      scope(["constant.numeric", "constant.language", "constant.character"], v.peach),
      scope(["keyword", "storage", "storage.type", "storage.modifier", "keyword.control"], v.mauve),
      scope(["keyword.operator", "punctuation", "meta.brace"], v.sub0),
      scope(["entity.name.function", "support.function", "meta.function-call.generic"], v.blue),
      scope(["entity.name.type", "entity.name.class", "support.type", "support.class", "entity.other.inherited-class"], v.yellow),
      scope(["variable", "variable.other", "meta.object-literal.key"], v.text),
      scope(["variable.parameter"], v.peach),
      scope(["entity.name.tag", "support.constant"], v.blue),
      scope(["entity.other.attribute-name"], v.yellow),
      scope(["markup.heading", "markup.bold"], v.blue, "bold"),
      scope(["markup.inserted", "markup.italic"], v.green),
      scope(["markup.deleted"], v.red),
    ],
  };
}

let hl: Highlighter | null = null;
let loading: Promise<void> | null = null;

export function ensureHighlighter(): Promise<void> {
  if (hl) return Promise.resolve();
  if (!loading) {
    // Register a theme per app theme up front so a runtime theme switch needs no
    // async reload — the matching theme is already loaded by id.
    loading = createHighlighter({ themes: THEMES.map(shikiThemeFor), langs: LANGS }).then((h) => {
      hl = h;
    });
  }
  return loading;
}

function shikiTheme(): string {
  return `agency-${currentThemeId()}`;
}

/** Returns inner HTML (a sequence of <span style>) for one line, or the
 *  HTML-escaped plain text if highlighting is unavailable. */
export async function highlightLine(code: string, lang: string): Promise<string> {
  try {
    await ensureHighlighter();
    if (!hl || lang === "text" || !LANGS.includes(lang)) return escapeHtml(code);
    const html = hl.codeToHtml(code, { lang, theme: shikiTheme() });
    // Extract inner spans of the single <span class="line">…</span>.
    const m = html.match(/<span class="line">(.*)<\/span>/s);
    return m ? m[1] : escapeHtml(code);
  } catch {
    return escapeHtml(code);
  }
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}
