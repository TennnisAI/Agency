import { useMemo } from "react";
import { marked } from "marked";

// Inline markdown renderer for PR descriptions and comment bodies. This content
// comes from GitHub users, so it is untrusted and must be sanitized before it
// touches the DOM.
//
// Two layers guard it: (1) `sanitize` below parses the HTML in an inert
// `<template>` (the browser's own parser — scripts never run there) and strips
// script/style/embed nodes, every `on*` event-handler attribute, and
// `javascript:` URLs; (2) the app CSP (`default-src 'self'`, no
// `script-src 'unsafe-inline'`, `img-src 'self' data:`) blocks inline script
// execution and off-host loads even if something slipped through. Rendering
// inline (vs. FileEditor's iframe preview) keeps many small comment bodies
// cheap. If the CSP is ever loosened, swap in DOMPurify here.
marked.setOptions({ gfm: true, breaks: true });

const DROP_TAGS = "script, style, iframe, object, embed, link, meta, base, form";

function sanitize(html: string): string {
  const tpl = document.createElement("template");
  tpl.innerHTML = html;
  tpl.content.querySelectorAll(DROP_TAGS).forEach((n) => n.remove());
  tpl.content.querySelectorAll("*").forEach((el) => {
    for (const attr of [...el.attributes]) {
      const name = attr.name.toLowerCase();
      const value = attr.value.replace(/\s/g, "").toLowerCase();
      if (name.startsWith("on")) el.removeAttribute(attr.name);
      else if ((name === "href" || name === "src" || name === "xlink:href") && value.startsWith("javascript:")) {
        el.removeAttribute(attr.name);
      }
    }
  });
  return tpl.innerHTML;
}

export default function Markdown({ text, className }: { text: string; className?: string }) {
  const html = useMemo(() => sanitize(marked.parse(text ?? "", { async: false }) as string), [text]);
  return <div className={`md${className ? ` ${className}` : ""}`} dangerouslySetInnerHTML={{ __html: html }} />;
}
