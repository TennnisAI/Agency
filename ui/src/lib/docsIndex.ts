import { BackendSearchHit, DocFile } from "../api";
import { parseDocsQuery } from "./docsQuery";

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
  /** Ordered frontmatter pairs as written (duplicates kept); [] when none. */
  frontmatter: [string, string][];
  /** First body line: the line after the closing fence, 0 when no frontmatter. */
  fmEnd: number;
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

// Frontmatter is a `---` fence on line 1, flat `key: value` lines, closing
// `---`. Deliberately forgiving: anything that doesn't fit (no closing fence
// nearby, a non-blank line that isn't `key: value`) means "no frontmatter" —
// the text is body, never an error. No YAML lists/nesting/multiline values.
const FM_KEY_RE = /^([A-Za-z][A-Za-z0-9_-]*)\s*:\s*(.*)$/;
const FM_MAX_LINES = 100;

function unquote(v: string): string {
  if (
    v.length >= 2 &&
    ((v.startsWith('"') && v.endsWith('"')) || (v.startsWith("'") && v.endsWith("'")))
  ) {
    return v.slice(1, -1);
  }
  return v;
}

/**
 * Parse leading frontmatter from a note's lines. Returns the ordered pairs
 * and `end` (the first body line, just past the closing fence), or null when
 * the note has no well-formed frontmatter. Shared with the live-preview
 * renderer so the editor and the index agree on the fence range.
 */
export function parseFrontmatter(
  lines: string[],
): { pairs: [string, string][]; end: number } | null {
  if (lines[0]?.trim() !== "---") return null;
  const pairs: [string, string][] = [];
  const limit = Math.min(lines.length, FM_MAX_LINES);
  for (let i = 1; i < limit; i++) {
    const line = lines[i];
    if (line.trim() === "---") return { pairs, end: i + 1 };
    if (!line.trim()) continue;
    const m = FM_KEY_RE.exec(line);
    if (!m) return null;
    pairs.push([m[1], unquote(m[2].trim())]);
  }
  return null;
}

/** Strip inline code spans so tags/links inside backticks don't index. */
function stripInlineCode(line: string): string {
  return line.replace(/`[^`]*`/g, (m) => " ".repeat(m.length));
}

/**
 * All wikilinks in a markdown text, skipping fenced blocks and inline code —
 * the same scan `parseDoc` uses for notes, exported standalone so issue
 * bodies (which live outside any corpus) can feed the cross-domain link
 * index (lib/links.ts).
 */
export function extractWikilinks(text: string): WikiLink[] {
  const links: WikiLink[] = [];
  let inFence = false;
  const lines = text.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const raw = lines[i];
    if (FENCE_RE.test(raw)) {
      inFence = !inFence;
      continue;
    }
    if (inFence) continue;
    for (const m of stripInlineCode(raw).matchAll(WIKILINK_RE)) {
      links.push({
        target: m[1].trim(),
        heading: m[2] ? m[2].slice(1).trim() || null : null,
        alias: m[3]?.trim() || null,
        line: i,
      });
    }
  }
  return links;
}

function parseDoc(file: DocFile): DocMeta {
  const base = stripExt(file.path.split("/").pop() ?? file.path);
  const headings: Heading[] = [];
  const tags: string[] = [];
  let title = "";
  let inFence = false;

  const lines = file.text.split("\n");
  const fm = parseFrontmatter(lines);
  const fmEnd = fm?.end ?? 0;
  // Nothing inside frontmatter is a heading, tag, or link.
  const links = extractWikilinks(file.text).filter((l) => l.line >= fmEnd);
  for (let i = fmEnd; i < lines.length; i++) {
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

    for (const m of stripInlineCode(raw).matchAll(TAG_RE)) {
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
    frontmatter: fm?.pairs ?? [],
    fmEnd,
    text: file.text,
    tooLarge: file.tooLarge,
  };
}

/**
 * Does this doc's frontmatter satisfy a `key:value` filter? Key match is
 * case-insensitive exact; value match is case-insensitive substring; an
 * empty filter value means "has this key".
 */
export function fmMatches(meta: DocMeta, key: string, value: string): boolean {
  const k = key.toLowerCase();
  const v = value.toLowerCase();
  for (const [pk, pv] of meta.frontmatter) {
    if (pk.toLowerCase() !== k) continue;
    if (!v || pv.toLowerCase().includes(v)) return true;
  }
  return false;
}

/** Paths whose frontmatter satisfies every filter. */
export function fmFilterPaths(index: DocsIndex, filters: [string, string][]): Set<string> {
  const out = new Set<string>();
  for (const d of index.docs.values()) {
    if (filters.every(([k, v]) => fmMatches(d, k, v))) out.add(d.path);
  }
  return out;
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

const MAX_SEARCH_HITS = 200;
const MAX_HITS_PER_DOC = 5;

/**
 * The index-local half of docs search: `#` queries match the tag index,
 * `key:value` tokens filter on frontmatter, the free-text remainder matches
 * titles (line -1 hits). A filter-only query returns every matching doc.
 * Body hits come from the backend search primitive and are folded in via
 * [`mergeBodyHits`] — this half stays synchronous so tag/title/filter
 * results never wait on IPC.
 */
export function searchLocal(index: DocsIndex, query: string): SearchHit[] {
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

  const { filters, text } = parseDocsQuery(q);
  for (const d of index.docs.values()) {
    if (filters.length && !filters.every(([k, v]) => fmMatches(d, k, v))) continue;
    if (text ? d.title.toLowerCase().includes(text) : filters.length > 0) {
      hits.push({ path: d.path, line: -1, snippet: d.title });
      if (hits.length >= MAX_SEARCH_HITS) break;
    }
  }
  return hits;
}

/**
 * Fold backend body hits (1-based lines, raw line text) into the local
 * title/tag hits: 0-based lines, trimmed snippets, only paths the index knows
 * (the corpus is the source of truth for what counts as a note), capped per
 * doc and in total so a common word can't flood the pane. When the query
 * carried `key:value` filters, `allowed` restricts body hits to the docs
 * that passed them (the backend only saw the free-text remainder).
 */
export function mergeBodyHits(
  index: DocsIndex,
  local: SearchHit[],
  body: BackendSearchHit[],
  allowed?: Set<string>,
): SearchHit[] {
  const hits = [...local];
  const perDoc = new Map<string, number>();
  for (const h of local) perDoc.set(h.path, (perDoc.get(h.path) ?? 0) + 1);
  for (const b of body) {
    if (hits.length >= MAX_SEARCH_HITS) break;
    if (!index.docs.has(b.path)) continue;
    if (allowed && !allowed.has(b.path)) continue;
    const n = perDoc.get(b.path) ?? 0;
    if (n >= MAX_HITS_PER_DOC) continue;
    hits.push({ path: b.path, line: b.line - 1, snippet: b.text.trim() });
    perDoc.set(b.path, n + 1);
  }
  return hits;
}

/**
 * Full-text search over the in-memory index. A query starting with "#"
 * matches the tag index; anything else is a case-insensitive substring search
 * over every doc's text, one hit per matching line (capped so a common word
 * can't flood the pane). Kept as the fallback when the backend search command
 * fails — the primary path is `searchLocal` + `mergeBodyHits`.
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
  const { filters, text } = parseDocsQuery(q);
  for (const d of index.docs.values()) {
    if (filters.length && !filters.every(([k, v]) => fmMatches(d, k, v))) continue;
    let inDoc = 0;
    if (text ? d.title.toLowerCase().includes(text) : filters.length > 0) {
      hits.push({ path: d.path, line: -1, snippet: d.title });
      inDoc++;
    }
    if (text) {
      const lines = d.text.split("\n");
      for (let i = 0; i < lines.length && inDoc < MAX_PER_DOC; i++) {
        if (lines[i].toLowerCase().includes(text)) {
          hits.push({ path: d.path, line: i, snippet: lines[i].trim() });
          inDoc++;
        }
      }
    }
    if (hits.length >= MAX_HITS) break;
  }
  return hits;
}
