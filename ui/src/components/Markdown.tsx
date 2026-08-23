import { useMemo } from "react";
import { onMarkdownLinkClick, renderMarkdown } from "../lib/mdHtml";

// Inline markdown renderer for PR descriptions and comment bodies. The
// sanitizing and the link handling live in lib/mdHtml, shared with the Files
// tab's preview mode; rendering inline (vs. a frame per body) keeps many small
// comment bodies cheap.
//
// `breaks`: a comment box submits the lines its author typed, so a bare newline
// is a line break here even though it is not one in a markdown file.
export default function Markdown({ text, className }: { text: string; className?: string }) {
  const html = useMemo(() => renderMarkdown(text, { breaks: true }), [text]);
  return (
    <div
      className={`md${className ? ` ${className}` : ""}`}
      onClick={onMarkdownLinkClick}
      dangerouslySetInnerHTML={{ __html: html }}
    />
  );
}
