import { Issue } from "../api";
import { CrossRefs, IssueRef } from "./links";
import { issueLabel } from "./issues";

// Issue-to-issue links: the `links:` frontmatter list, resolved for the detail
// pane's Links section.
//
// Storage is a list of issue keys on each issue ("AGE-12"), any project's — see
// issuefs.rs. The relation is undirected, and the app writes both sides when a
// link is made here, but a file edited by hand may only say it on one side, so
// what the pane shows is the union: this issue's own list, plus every issue
// whose list names this one. Keys are the stored form (an issue can be linked
// before its file exists, or across a tracker this workspace can't see), so
// resolution to a real issue happens at render time and may come back empty.

/** One row of the Links section. */
export interface IssueLink {
  /** "AGE-12" — the stored key, and the row's own label. */
  label: string;
  /** The issue behind the key, or null when nothing loaded matches it. */
  ref: IssueRef | null;
  /** Whether this issue's own `links:` names it (as opposed to only the other's). */
  outgoing: boolean;
}

/** The stored form of a key: trimmed and upper-cased, as the file holds it. */
export function normalizeLinkKey(key: string): string {
  return key.trim().toUpperCase();
}

const same = (a: string, b: string) => normalizeLinkKey(a) === normalizeLinkKey(b);

/** Whether `links` already names `key`, however either side is cased. */
export function hasLink(links: string[], key: string): boolean {
  return links.some((l) => same(l, key));
}

/** `links` with `key` added (no-op if already there); order is preserved. */
export function withLink(links: string[], key: string): string[] {
  const next = normalizeLinkKey(key);
  return links.some((l) => same(l, next)) ? [...links] : [...links, next];
}

/** `links` without `key`. */
export function withoutLink(links: string[], key: string): string[] {
  return links.filter((l) => !same(l, key));
}

/**
 * The Links section's rows: this issue's own links first, in file order, then
 * the issues that link to it from their side only, by project and number.
 * `label` is this issue's key, so its own list can't name itself.
 */
export function issueLinks(
  issue: Pick<Issue, "id" | "links">,
  label: string,
  cross: CrossRefs | null,
): IssueLink[] {
  const out: IssueLink[] = [];
  const seen = new Set<string>();
  for (const raw of issue.links) {
    const key = normalizeLinkKey(raw);
    if (seen.has(key) || same(key, label)) continue;
    seen.add(key);
    out.push({ label: key, ref: cross?.issuesByLabel.get(key.toLowerCase()) ?? null, outgoing: true });
  }
  if (!cross) return out;
  const inbound: IssueLink[] = [];
  for (const ref of cross.issuesById.values()) {
    if (ref.issue.id === issue.id) continue;
    const key = issueLabel(ref.project, ref.issue);
    if (seen.has(normalizeLinkKey(key))) continue;
    if (!ref.issue.links.some((l) => same(l, label))) continue;
    inbound.push({ label: key, ref, outgoing: false });
  }
  inbound.sort((a, b) => a.label.localeCompare(b.label, undefined, { numeric: true }));
  return [...out, ...inbound];
}

/**
 * Issues that can still be linked to: everything loaded except this one and
 * the ones already linked. This project's issues first (a link usually stays
 * inside a tracker), newest first within each — the picker searches over the
 * rest.
 */
export function linkCandidates(
  cross: CrossRefs | null,
  projectId: string,
  label: string,
  existing: IssueLink[],
): IssueRef[] {
  if (!cross) return [];
  const taken = new Set(existing.map((l) => normalizeLinkKey(l.label)));
  taken.add(normalizeLinkKey(label));
  return [...cross.issuesById.values()]
    .filter((ref) => !taken.has(normalizeLinkKey(issueLabel(ref.project, ref.issue))))
    .sort((a, b) => {
      const own = Number(b.project.id === projectId) - Number(a.project.id === projectId);
      if (own !== 0) return own;
      return a.project.name.localeCompare(b.project.name) || b.issue.seq - a.issue.seq;
    });
}
