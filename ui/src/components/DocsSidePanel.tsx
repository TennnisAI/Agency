import { useState } from "react";
import { DocsIndex } from "../lib/docsIndex";

/**
 * Right-hand panel for the Docs tab: the open note's outline (click to jump)
 * and its backlinks ("linked mentions", click to navigate). Both read the
 * index, so they lag unsaved edits by at most autosave + one poll (~3s).
 */
export default function DocsSidePanel({
  index, selected, onJumpToHeading, onOpen,
}: {
  index: DocsIndex | null;
  selected: string | null;
  onJumpToHeading: (text: string) => void;
  onOpen: (path: string) => void;
}) {
  const [outlineOpen, setOutlineOpen] = useState(true);
  const [backlinksOpen, setBacklinksOpen] = useState(true);

  const doc = selected && index ? index.docs.get(selected) : undefined;
  const backlinks = selected && index ? index.backlinks.get(selected) ?? [] : [];

  return (
    <div className="docs-side-body">
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
    </div>
  );
}
