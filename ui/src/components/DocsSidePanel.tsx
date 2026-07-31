import { useState } from "react";
import { DocsIndex } from "../lib/docsIndex";
import { LinkEdge } from "../lib/links";

/**
 * Right-hand panel for the Docs tab: the open note's frontmatter properties
 * (click a value to filter the search), its outline (click to jump), its
 * backlinks ("linked mentions", click to navigate), and the issues whose
 * bodies link to it (Mentions, click to jump to the tracker). All read
 * polled indexes, so they lag unsaved edits by at most autosave + one poll.
 */
export default function DocsSidePanel({
  index, selected, mentions, onJumpToHeading, onOpen, onOpenMention, onFilter,
}: {
  index: DocsIndex | null;
  selected: string | null;
  mentions: LinkEdge[];
  onJumpToHeading: (text: string) => void;
  onOpen: (path: string) => void;
  onOpenMention: (edge: LinkEdge) => void;
  onFilter: (key: string, value: string) => void;
}) {
  const [propsOpen, setPropsOpen] = useState(true);
  const [outlineOpen, setOutlineOpen] = useState(true);
  const [backlinksOpen, setBacklinksOpen] = useState(true);
  const [mentionsOpen, setMentionsOpen] = useState(true);

  const doc = selected && index ? index.docs.get(selected) : undefined;
  const backlinks = selected && index ? index.backlinks.get(selected) ?? [] : [];

  return (
    <div className="docs-side-body">
      {doc && doc.frontmatter.length > 0 && (
        <>
          <button className="docs-side-head" onClick={() => setPropsOpen((o) => !o)}>
            <span className="docs-side-chev">{propsOpen ? "▾" : "▸"}</span> Properties
          </button>
          {propsOpen && (
            <div className="docs-side-section">
              {doc.frontmatter.map(([k, v], i) => (
                <div key={`${k}:${i}`} className="docs-prop-row">
                  <span className="docs-prop-key">{k}</span>
                  <span
                    className="docs-prop-value"
                    title={`Filter ${k}:${v}`}
                    onClick={() => onFilter(k, v)}
                  >
                    {v || "…"}
                  </span>
                </div>
              ))}
            </div>
          )}
        </>
      )}

      <button className="docs-side-head" onClick={() => setOutlineOpen((o) => !o)}>
        <span className="docs-side-chev">{outlineOpen ? "▾" : "▸"}</span> Outline
      </button>
      {outlineOpen && (
        <div className="docs-side-section">
          {!doc || doc.headings.length === 0 ? (
            <div className="docs-side-none">No headings</div>
          ) : (
            doc.headings.map((h, i) => (
              <div
                key={`${h.line}:${i}`}
                className="docs-outline-row"
                style={{ paddingLeft: 10 + (h.level - 1) * 12 }}
                onClick={() => onJumpToHeading(h.text)}
              >
                {h.text}
              </div>
            ))
          )}
        </div>
      )}

      <button className="docs-side-head" onClick={() => setBacklinksOpen((o) => !o)}>
        <span className="docs-side-chev">{backlinksOpen ? "▾" : "▸"}</span> Backlinks
        {backlinks.length > 0 && <span className="docs-side-count">{backlinks.length}</span>}
      </button>
      {backlinksOpen && (
        <div className="docs-side-section">
          {backlinks.length === 0 ? (
            <div className="docs-side-none">No backlinks</div>
          ) : (
            backlinks.map((b, i) => (
              <div key={`${b.from}:${b.line}:${i}`} className="docs-backlink-row" onClick={() => onOpen(b.from)}>
                <span className="docs-backlink-title">{index?.docs.get(b.from)?.title ?? b.from}</span>
                <span className="docs-backlink-snippet">{b.snippet}</span>
              </div>
            ))
          )}
        </div>
      )}

      <button className="docs-side-head" onClick={() => setMentionsOpen((o) => !o)}>
        <span className="docs-side-chev">{mentionsOpen ? "▾" : "▸"}</span> Mentions
        {mentions.length > 0 && <span className="docs-side-count">{mentions.length}</span>}
      </button>
      {mentionsOpen && (
        <div className="docs-side-section">
          {mentions.length === 0 ? (
            <div className="docs-side-none">No issues link here</div>
          ) : (
            mentions.map((m, i) => (
              <div key={`${m.fromId}:${m.line}:${i}`} className="docs-backlink-row" onClick={() => onOpenMention(m)}>
                <span className="docs-backlink-title">
                  <span className="mention-glyph" aria-hidden>▧</span>
                  {m.fromLabel ? `${m.fromLabel} ` : ""}{m.fromTitle}
                </span>
                <span className="docs-backlink-snippet">{m.snippet}</span>
              </div>
            ))
          )}
        </div>
      )}
    </div>
  );
}
