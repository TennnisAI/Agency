import type { MouseEvent } from "react";
import DOMPurify from "dompurify";
import { marked } from "marked";
import { openUrl } from "@tauri-apps/plugin-opener";

// Markdown rendered straight into the app's own document: PR descriptions and
// comment bodies (components/Markdown) and the Files tab's preview mode
// (components/FileEditor).
//
// Both sources are untrusted — GitHub users write the first, agents write the
// second — so two layers guard the HTML: (1) DOMPurify strips scripts,
// event-handler attributes, and dangerous URL schemes with a maintained
// allowlist (far more robust than a hand-rolled denylist against mutation-XSS
// and namespace-confusion vectors); (2) the app CSP (`default-src 'self'`, no
// `script-src 'unsafe-inline'`, `img-src 'self' data:`) blocks inline script
// execution and off-host loads even if something slipped through.

/**
 * Lift marked's `<code class="language-rust">` onto the enclosing `<pre>`, so
 * CSS can label the block the way the docs live preview does. The language is
 * re-checked against a conservative charset here — it lands in an attribute,
 * and what this builds is a document, not a template.
 */
export function labelCodeLangs(html: string): string {
  return html.replace(
    /<pre><code class="language-([A-Za-z0-9_+#.-]+)"/g,
    (_m, lang: string) => `<pre data-lang="${lang}"><code class="language-${lang}"`,
  );
}

export interface RenderOpts {
  /**
   * Render a bare newline as a line break. On for chat-like bodies (a comment
   * box submits the lines its author typed); off for markdown files, which
   * hard-wrap their source and expect a blank line to start a paragraph.
   */
  breaks?: boolean;
  /** Tag fenced blocks with their language, for the CSS label. */
  labelFences?: boolean;
}

/**
 * Markdown to sanitized HTML. Options are passed per call and never through
 * `marked.setOptions`: those are process-global, so a `breaks: true` set for
 * comment bodies reached the file preview too — as a side effect of whether
 * that module had been imported yet, which is not something a render should
 * depend on.
 */
export function renderMarkdown(text: string, opts: RenderOpts = {}): string {
  const parsed = marked.parse(text ?? "", {
    async: false,
    gfm: true,
    breaks: opts.breaks ?? false,
  }) as string;
  const html = opts.labelFences ? labelCodeLangs(parsed) : parsed;
  return DOMPurify.sanitize(html, { USE_PROFILES: { html: true } });
}

/** Schemes handed to the browser. Everything else in rendered markdown is inert. */
export function isExternalHref(href: string): boolean {
  return /^(https?|mailto):/i.test(href);
}

/**
 * A link in rendered markdown must not navigate the webview: the app is the
 * page, and following a link inside it replaces the whole UI with no way back.
 * http(s) and mailto open in the user's browser (the same route the docs live
 * preview takes).
 */
export function onMarkdownLinkClick(e: MouseEvent<HTMLElement>) {
  const a = (e.target as HTMLElement).closest("a");
  const href = a?.getAttribute("href");
  if (!a || !href) return;
  e.preventDefault();
  if (isExternalHref(href)) void openUrl(href).catch(() => {});
}
