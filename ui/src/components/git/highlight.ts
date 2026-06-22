import { createHighlighter, type Highlighter } from "shiki";

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

let hl: Highlighter | null = null;
let loading: Promise<void> | null = null;

export function ensureHighlighter(): Promise<void> {
  if (hl) return Promise.resolve();
  if (!loading) {
    loading = createHighlighter({ themes: ["catppuccin-mocha"], langs: LANGS }).then((h) => {
      hl = h;
    });
  }
  return loading;
}

/** Returns inner HTML (a sequence of <span style>) for one line, or the
 *  HTML-escaped plain text if highlighting is unavailable. */
export async function highlightLine(code: string, lang: string): Promise<string> {
  try {
    await ensureHighlighter();
    if (!hl || lang === "text" || !LANGS.includes(lang)) return escapeHtml(code);
    const html = hl.codeToHtml(code, { lang, theme: "catppuccin-mocha" });
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
