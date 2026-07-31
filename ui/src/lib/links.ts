import { Issue, Project, RunInfo } from "../api";
import { DocsIndex, extractWikilinks, resolveLink } from "./docsIndex";
import { issueLabel } from "./issues";

// Cross-domain links (one-stop Phase 7): the pure model behind typed wikilink
// targets ([[AGE-14]], [[run:<id>]]) and the mentions panels. Everything here
// is an in-memory structure rebuilt from data the frontend already polls —
// note corpora (useDocs), issue index rows (list_issues carries full bodies),
// and runs (list_runs) — the frontend sibling of docsIndex's backlinks map.

// Narrow structural views so tests (and the palette-style fan-out) can feed
// this without fabricating full API objects.
export type LinkProject = Pick<Project, "id" | "name" | "issue_key">;
export type LinkIssue = Pick<Issue, "id" | "projectId" | "seq" | "title" | "body" | "status">;
export type LinkRun = Pick<RunInfo, "id" | "projectId" | "issueId">;

/** One project's rows, as fetched by the useCrossRefs fan-out. */
export interface CrossSource {
  project: LinkProject;
  issues: LinkIssue[];
  runs: LinkRun[];
}

export interface IssueRef {
  project: LinkProject;
  issue: LinkIssue;
}

export interface RunRef {
  project: LinkProject;
  run: LinkRun;
}

/** Cross-project lookup tables for typed wikilink targets. */
export interface CrossRefs {
  /** lowercased "age-14" → the issue, across every project. */
  issuesByLabel: Map<string, IssueRef>;
  issuesById: Map<string, IssueRef>;
  runsById: Map<string, RunRef>;
  /** lowercased issue keys of every project ("age"), gating the issue domain. */
  keyPrefixes: Set<string>;
}

// The raw KEY-n shape. Loose on purpose (real keys are ~3 letters): whether a
// match *means* an issue is decided by the known-prefix check in
// resolveTarget, so hyphenated note names ("2026-07-31" starts with a digit
// and never matches; "FOO-3" matches but falls back to the note domain when
// no project uses the FOO key).
export const ISSUE_TARGET_RE = /^([A-Za-z][A-Za-z0-9]{1,7})-(\d+)$/;

const RUN_PREFIX = "run:";

export type TargetClass =
  | { kind: "run"; id: string }
  | { kind: "issuePattern"; prefix: string; label: string }
  | { kind: "note" };

/** Syntactic classification of a wikilink target (no lookups). */
export function classifyTarget(target: string): TargetClass {
  const t = target.trim();
  if (t.toLowerCase().startsWith(RUN_PREFIX)) {
    return { kind: "run", id: t.slice(RUN_PREFIX.length).trim() };
  }
  const m = ISSUE_TARGET_RE.exec(t);
  if (m) return { kind: "issuePattern", prefix: m[1].toLowerCase(), label: t.toLowerCase() };
  return { kind: "note" };
}

export function buildCrossRefs(sources: CrossSource[]): CrossRefs {
  const cross: CrossRefs = {
    issuesByLabel: new Map(),
    issuesById: new Map(),
    runsById: new Map(),
    keyPrefixes: new Set(),
  };
  for (const { project, issues, runs } of sources) {
    if (project.issue_key) cross.keyPrefixes.add(project.issue_key.toLowerCase());
    for (const issue of issues) {
      const ref = { project, issue };
      cross.issuesByLabel.set(issueLabel(project, issue).toLowerCase(), ref);
      cross.issuesById.set(issue.id, ref);
    }
    for (const run of runs) {
      cross.runsById.set(run.id, { project, run });
    }
  }
  return cross;
}

export type Resolution =
  | { kind: "note"; path: string }
  | { kind: "issue"; ref: IssueRef }
  | { kind: "run"; ref: RunRef }
  // A KEY-n target inside a known project's key namespace, but no such issue.
  // Deliberately never falls back to note creation: within a key namespace,
  // targets mean issues, and a note named "AGE-99.md" would shadow the
  // tracker.
  | { kind: "unresolvedIssue"; label: string }
  | { kind: "unresolvedRun"; id: string }
  | { kind: "unresolvedNote" };

/**
 * Resolve a wikilink target across the three domains. `cross` may be null
 * while the fan-out poll hasn't landed — typed targets then degrade to the
 * note domain (issue-pattern) or resolve optimistically (run:), so nothing
 * flashes unresolved during load.
 */
export function resolveTarget(
  index: DocsIndex | null,
  cross: CrossRefs | null,
  target: string,
): Resolution {
  const c = classifyTarget(target);
  if (c.kind === "run") {
    if (!cross) return { kind: "unresolvedRun", id: c.id };
    const ref = cross.runsById.get(c.id);
    return ref ? { kind: "run", ref } : { kind: "unresolvedRun", id: c.id };
  }
  if (c.kind === "issuePattern" && cross && cross.keyPrefixes.has(c.prefix)) {
    const ref = cross.issuesByLabel.get(c.label);
    return ref ? { kind: "issue", ref } : { kind: "unresolvedIssue", label: target.trim().toUpperCase() };
  }
  const path = index ? resolveLink(index, target) : null;
  return path ? { kind: "note", path } : { kind: "unresolvedNote" };
}

/**
 * What the live-preview decoration needs per wikilink: the unresolved flag
 * and a tooltip. Loading states (null index/cross) never flag unresolved.
 */
export function wikilinkView(
  index: DocsIndex | null,
  cross: CrossRefs | null,
  target: string,
): { unresolved: boolean; title: string } {
  const res = resolveTarget(index, cross, target);
  switch (res.kind) {
    case "issue":
      return { unresolved: false, title: `⌘-click to open ${issueLabel(res.ref.project, res.ref.issue)}` };
    case "run":
      return { unresolved: false, title: "⌘-click to open this run" };
    case "unresolvedIssue":
      return { unresolved: true, title: "No matching issue" };
    case "unresolvedRun":
      // cross === null is "still loading", not "missing".
      return cross ? { unresolved: true, title: "No matching run" } : { unresolved: false, title: "⌘-click to open this run" };
    case "unresolvedNote":
      return index ? { unresolved: true, title: "⌘-click to create" } : { unresolved: false, title: "⌘-click to open" };
    case "note":
      return { unresolved: false, title: "⌘-click to open" };
  }
}

// ── The links table ──────────────────────────────────────────────────────────

export type LinkKind = "note" | "issue" | "run";

/**
 * One cross-domain edge, keyed by its target. Note ids are
 * `${projectId}:${corpus-relative path}`; issues are their uuid (stable
 * across reconcile); runs are their uuid. Note→note stays in
 * DocsIndex.backlinks — this table holds only edges that cross a domain.
 */
export interface LinkEdge {
  fromKind: "note" | "issue";
  fromProjectId: string;
  /** Note path (corpus-relative) or issue uuid. */
  fromId: string;
  /** Doc title, or the issue title (label rides separately). */
  fromTitle: string;
  /** "AGE-14" when the source is an issue. */
  fromLabel: string | null;
  line: number;
  snippet: string;
  toKind: LinkKind;
  toId: string;
}

export const linkKey = (kind: LinkKind, id: string) => `${kind}:${id}`;
export const noteId = (projectId: string, path: string) => `${projectId}:${path}`;

export interface Corpus {
  project: LinkProject;
  index: DocsIndex;
}

/**
 * Build the cross-domain links table from the loaded corpora and the
 * cross-project issue rows. Only resolved edges enter the table; self-links
 * are dropped. Issue-body note targets resolve against the issue's own
 * project corpus first, then the other loaded corpora.
 */
export function buildLinkIndex(corpora: Corpus[], cross: CrossRefs): Map<string, LinkEdge[]> {
  const table = new Map<string, LinkEdge[]>();
  const add = (edge: LinkEdge) => {
    const key = linkKey(edge.toKind, edge.toId);
    const list = table.get(key) ?? [];
    list.push(edge);
    table.set(key, list);
  };

  for (const { project, index } of corpora) {
    for (const doc of index.docs.values()) {
      for (const link of doc.links) {
        const res = resolveTarget(index, cross, link.target);
        if (res.kind !== "issue" && res.kind !== "run") continue; // note→note lives in backlinks
        add({
          fromKind: "note",
          fromProjectId: project.id,
          fromId: doc.path,
          fromTitle: doc.title,
          fromLabel: null,
          line: link.line,
          snippet: (doc.text.split("\n")[link.line] ?? "").trim(),
          toKind: res.kind,
          toId: res.kind === "issue" ? res.ref.issue.id : res.ref.run.id,
        });
      }
    }
  }

  for (const { project, issue } of cross.issuesById.values()) {
    if (!issue.body) continue;
    const lines = issue.body.split("\n");
    // Own corpus first so [[Setup]] in AGE-14's body means AGE's docs even
    // when the workspace also has a Setup note.
    const ordered = [...corpora].sort((a, b) =>
      Number(b.project.id === issue.projectId) - Number(a.project.id === issue.projectId));
    for (const link of extractWikilinks(issue.body)) {
      let edge: { toKind: LinkKind; toId: string } | null = null;
      const c = classifyTarget(link.target);
      if (c.kind === "run") {
        const ref = cross.runsById.get(c.id);
        if (ref) edge = { toKind: "run", toId: ref.run.id };
      } else if (c.kind === "issuePattern" && cross.keyPrefixes.has(c.prefix)) {
        const ref = cross.issuesByLabel.get(c.label);
        if (ref && ref.issue.id !== issue.id) edge = { toKind: "issue", toId: ref.issue.id };
      } else {
        for (const corpus of ordered) {
          const path = resolveLink(corpus.index, link.target);
          if (path) {
            edge = { toKind: "note", toId: noteId(corpus.project.id, path) };
            break;
          }
        }
      }
      if (!edge) continue;
      add({
        fromKind: "issue",
        fromProjectId: issue.projectId,
        fromId: issue.id,
        fromTitle: issue.title,
        fromLabel: issueLabel(project, issue),
        line: link.line,
        snippet: (lines[link.line] ?? "").trim(),
        ...edge,
      });
    }
  }

  return table;
}

export function mentionsOf(table: Map<string, LinkEdge[]>, kind: LinkKind, id: string): LinkEdge[] {
  return table.get(linkKey(kind, id)) ?? [];
}

/**
 * Issue entries for the `[[` completion: label "AGE-14", detail = title.
 * Sorted by project then seq; closed issues included (linking a done issue
 * from a retro note is normal).
 */
export function issueCompletionOptions(cross: CrossRefs): { label: string; detail: string }[] {
  return [...cross.issuesById.values()]
    .sort((a, b) =>
      a.project.name.localeCompare(b.project.name) ||
      a.issue.seq - b.issue.seq)
    .map((ref) => ({ label: issueLabel(ref.project, ref.issue), detail: ref.issue.title }));
}
