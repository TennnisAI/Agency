import { DocFile } from "../api";

// The docs index: everything the Docs tab derives from the markdown corpus —
// titles, headings, tags, wikilinks, backlinks, and the text kept for search.
// Built fresh from `readDocsCorpus` output; parsing is regex-over-lines with a
// fenced-code state flag so it stays cheap enough to rerun on every poll.

export interface Heading {
  level: number;
  text: string;
  line: number; // 0-based
}

export interface WikiLink {
  target: string; // raw target as written, e.g. "guides/Setup"
  heading: string | null; // "#Heading" part without the #, if present
  alias: string | null; // display alias after |, if present
  line: number;
}

export interface DocMeta {
  path: string; // rel to docs dir, "/"-separated
  base: string; // basename without extension
  title: string; // first H1, else base
  headings: Heading[];
  tags: string[];
  links: WikiLink[];
  text: string;
  tooLarge: boolean;
}

export interface Backlink {
  from: string; // path of the referring doc
  line: number;
  snippet: string;
}

export interface DocsIndex {
  docs: Map<string, DocMeta>;
  /** lowercased basename → paths that have it (shortest path first). */
  byBase: Map<string, string[]>;
  /** resolved target path → references pointing at it. */
  backlinks: Map<string, Backlink[]>;
  /** lowercased tag → paths that mention it. */
  tags: Map<string, string[]>;
}

export const WIKILINK_RE = /\[\[([^\]|#\n]+)(#[^\]|\n]*)?(?:\|([^\]\n]+))?\]\]/g;
const TAG_RE = /(^|[\s(])#([A-Za-z0-9_][A-Za-z0-9_/-]*)/g;
const HEADING_RE = /^(#{1,6})\s+(.+)$/;
const FENCE_RE = /^(```|~~~)/;

export function stripExt(name: string): string {
  return name.replace(/\.(md|markdown)$/i, "");
}

/** Strip inline code spans so tags/links inside backticks don't index. */
function stripInlineCode(line: string): string {
  return line.replace(/`[^`]*`/g, (m) => " ".repeat(m.length));
}

function parseDoc(file: DocFile): DocMeta {
  const base = stripExt(file.path.split("/").pop() ?? file.path);
  const headings: Heading[] = [];
  const tags: string[] = [];
  const links: WikiLink[] = [];
  let title = "";
  let inFence = false;

  const lines = file.text.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    if (FENCE_RE.test(raw)) {
      inFence = !inFence;
      continue;
    }
    if (inFence) continue;

    const h = HEADING_RE.exec(raw);
    if (h) {
      const text = h[2].trim();
      headings.push({ level: h[1].length, text, line: i });
      if (!title && h[1].length === 1) title = text;
    }

    const line = stripInlineCode(raw);
    for (const m of line.matchAll(WIKILINK_RE)) {
      links.push({
        target: m[1].trim(),
        heading: m[2] ? m[2].slice(1).trim() || null : null,
        alias: m[3]?.trim() || null,
        line: i,
      });
    }
    for (const m of line.matchAll(TAG_RE)) {
      const tag = m[2].toLowerCase();
      if (!tags.includes(tag)) tags.push(tag);
    }
  }

  return {
    path: file.path,
    base,
    title: title || base,
    headings,
    tags,
    links,
    text: file.text,
    tooLarge: file.tooLarge,
  };
}

/**
 * Resolve a wikilink target against the index:
 *  - "Note" matches any doc whose basename equals it case-insensitively
 *    (ambiguity → the shortest path wins);
 *  - "folder/Note" matches by case-insensitive path suffix.
 * Returns the doc path or null when unresolved.
 */
export function resolveLink(index: DocsIndex, target: string): string | null {
  const t = target.trim().toLowerCase();
  if (!t) return null;
  if (!t.includes("/")) {
    const paths = index.byBase.get(t);
    return paths?.[0] ?? null;
  }
  const suffix = t.endsWith(".md") || t.endsWith(".markdown") ? t : null;
  let best: string | null = null;
  for (const path of index.docs.keys()) {
    const p = path.toLowerCase();
    const stripped = stripExt(p);
    if (stripped === t || stripped.endsWith("/" + t) || (suffix && (p === suffix || p.endsWith("/" + suffix)))) {
      if (best === null || path.length < best.length) best = path;
    }
  }
  return best;
}

export function buildIndex(files: DocFile[]): DocsIndex {
  const docs = new Map<string, DocMeta>();
  for (const f of files) docs.set(f.path, parseDoc(f));

  const byBase = new Map<string, string[]>();
  for (const d of docs.values()) {
    const key = d.base.toLowerCase();
    const list = byBase.get(key) ?? [];
    list.push(d.path);
    byBase.set(key, list);
  }
  // Shortest path first so ambiguous [[Note]] links resolve deterministically.
  for (const list of byBase.values()) {
    list.sort((a, b) => a.length - b.length || a.localeCompare(b));
  }

  const index: DocsIndex = { docs, byBase, backlinks: new Map(), tags: new Map() };

  for (const d of docs.values()) {
    for (const link of d.links) {
      const resolved = resolveLink(index, link.target);
      if (!resolved || resolved === d.path) continue;
      const list = index.backlinks.get(resolved) ?? [];
      const snippet = (d.text.split("\n")[link.line] ?? "").trim();
      list.push({ from: d.path, line: link.line, snippet });
      index.backlinks.set(resolved, list);
    }
    for (const tag of d.tags) {
      const list = index.tags.get(tag) ?? [];
      list.push(d.path);
      index.tags.set(tag, list);
    }
  }

  return index;
}

export interface SearchHit {
  path: string;
  line: number; // 0-based; -1 for a title-only match
  snippet: string;
}

/**
 * Full-text search. A query starting with "#" matches the tag index; anything
 * else is a case-insensitive substring search over every doc's text, one hit
 * per matching line (capped so a common word can't flood the pane).
 */
export function searchDocs(index: DocsIndex, query: string): SearchHit[] {
  const q = query.trim().toLowerCase();
  if (!q) return [];
  const hits: SearchHit[] = [];

  if (q.startsWith("#") && q.length > 1) {
    const tag = q.slice(1);
    for (const [t, paths] of index.tags) {
      if (!t.startsWith(tag)) continue;
      for (const path of paths) {
        if (!hits.some((h) => h.path === path)) {
          hits.push({ path, line: -1, snippet: `#${t}` });
        }
      }
    }
    return hits;
  }

  const MAX_HITS = 200;
  const MAX_PER_DOC = 5;
  for (const d of index.docs.values()) {
    let inDoc = 0;
    if (d.title.toLowerCase().includes(q)) {
      hits.push({ path: d.path, line: -1, snippet: d.title });
      inDoc++;
    }
    const lines = d.text.split("\n");
    for (let i = 0; i < lines.length && inDoc < MAX_PER_DOC; i++) {
      if (lines[i].toLowerCase().includes(q)) {
        hits.push({ path: d.path, line: i, snippet: lines[i].trim() });
        inDoc++;
      }
    }
    if (hits.length >= MAX_HITS) break;
  }
  return hits;
}
