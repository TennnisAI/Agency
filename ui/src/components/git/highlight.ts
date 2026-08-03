import { bundledLanguages, createHighlighter, type BundledLanguage, type Highlighter, type ThemeRegistrationRaw } from "shiki";
import { currentThemeId, THEMES, type Theme } from "../../lib/themes";
import { languageIdForPath } from "../../lib/fileLanguage";

/** Shiki language id for a diffed file, or "text" when it has no grammar.
 *  Shiki carries VS Code's own TextMate grammars, so the ids in
 *  lib/fileLanguage.ts are the ids it is keyed by. */
export function langForPath(path: string): string {
  const id = languageIdForPath(path);
  return id && id in bundledLanguages ? id : "text";
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
    // async reload — the matching theme is already loaded by id. Grammars, by
    // contrast, are fetched per language on first use: there are 200-odd of
    // them, and loading the set eagerly would cost more than every diff the app
    // will ever show.
    loading = createHighlighter({ themes: THEMES.map(shikiThemeFor), langs: [] }).then((h) => {
      hl = h;
    });
  }
  return loading;
}

// One load attempt per language, shared by every line that needs it.
const grammars = new Map<string, Promise<boolean>>();

function ensureGrammar(lang: string): Promise<boolean> {
  let p = grammars.get(lang);
  if (!p) {
    // A failure sticks for the session on purpose: every line of a diff asks
    // for the same language in turn, so retrying would mean thousands of
    // failed fetches for one file instead of one.
    p = hl!.loadLanguage(lang as BundledLanguage).then(() => true, () => false);
    grammars.set(lang, p);
  }
  return p;
}

function shikiTheme(): string {
  return `agency-${currentThemeId()}`;
}

/** Returns inner HTML (a sequence of <span style>) for one line, or the
 *  HTML-escaped plain text if highlighting is unavailable. */
export async function highlightLine(code: string, lang: string): Promise<string> {
  try {
    await ensureHighlighter();
    if (!hl || lang === "text" || !(lang in bundledLanguages)) return escapeHtml(code);
    if (!(await ensureGrammar(lang))) return escapeHtml(code);
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
