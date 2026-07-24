import { useMemo } from "react";
import { marked } from "marked";
import DOMPurify from "dompurify";

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

export default function Markdown({ text, className }: { text: string; className?: string }) {
  const html = useMemo(() => sanitize(marked.parse(text ?? "", { async: false }) as string), [text]);
  return <div className={`md${className ? ` ${className}` : ""}`} dangerouslySetInnerHTML={{ __html: html }} />;
}
