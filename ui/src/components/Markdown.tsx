import { useMemo } from "react";
import { marked } from "marked";
import DOMPurify from "dompurify";
import { openUrl } from "@tauri-apps/plugin-opener";

// Inline markdown renderer for PR descriptions and comment bodies. This content
// comes from GitHub users, so it is untrusted and must be sanitized before it
// touches the DOM.
//
// Two layers guard it: (1) DOMPurify strips scripts, event-handler attributes,
// and dangerous URL schemes with a maintained allowlist (far more robust than a
// hand-rolled denylist against mutation-XSS and namespace-confusion vectors);
// (2) the app CSP (`default-src 'self'`, no `script-src 'unsafe-inline'`,
// `img-src 'self' data:`) blocks inline script execution and off-host loads even
// if something slipped through. Rendering inline (vs. FileEditor's iframe
// preview) keeps many small comment bodies cheap.
marked.setOptions({ gfm: true, breaks: true });

function sanitize(html: string): string {
  return DOMPurify.sanitize(html, { USE_PROFILES: { html: true } });
}

// A link in rendered markdown must not navigate the webview: the app is the
// page, and following a link inside it replaces the whole UI with no way back.
// http(s) and mailto open in the user's browser (the same route the live
// preview takes); anything else is inert.
function onLinkClick(e: React.MouseEvent<HTMLDivElement>) {
  const a = (e.target as HTMLElement).closest("a");
  const href = a?.getAttribute("href");
  if (!a || !href) return;
  e.preventDefault();
  if (/^(https?|mailto):/i.test(href)) void openUrl(href).catch(() => {});
}

export default function Markdown({ text, className }: { text: string; className?: string }) {
  const html = useMemo(() => sanitize(marked.parse(text ?? "", { async: false }) as string), [text]);
  return (
    <div
      className={`md${className ? ` ${className}` : ""}`}
      onClick={onLinkClick}
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}
