/**
 * The Settings screen's table of contents, and the search over it.
 *
 * Settings grew to thirteen sections on one scroll, and the thing nobody could
 * tell from that page was which of them applied to every project and which only
 * to the one selected (AGE-187). So the sections are grouped, each group
 * carries its scope, and the scope is stated again on the section itself.
 *
 * The index lives here rather than in the component because it is the whole of
 * the searchable content: matching is a pure function of a query, this table,
 * and the handful of runtime names the caller passes in.
 */

export type Scope = "global" | "project";

/**
 * Every id, as a value rather than only as a type. `SECTION_BY_ID` and
 * `GROUP_BY_ID` are built with an `as` cast, so an id the table has no row for
 * is a hole TypeScript cannot see: `scopeOf` and the section heading both read
 * straight through it and throw while rendering that section. The types below
 * are derived from these lists, and a test asserts each table covers its list,
 * so the hole is a failing test rather than a blank screen.
 */
export const GROUP_IDS = ["interface", "agents", "project", "app"] as const;

export const SECTION_IDS = [
  "appearance", "editor", "messages", "notifications",
  "agents", "mcp", "localModel",
  "backlog", "knowledge", "files",
  "workspace", "diagnostics", "about",
] as const;

export type GroupId = (typeof GROUP_IDS)[number];

export type SectionId = (typeof SECTION_IDS)[number];

export interface Group {
  id: GroupId;
  label: string;
  scope: Scope;
}

export interface Section {
  id: SectionId;
  label: string;
  group: GroupId;
  /**
   * What search matches on beyond the label. The sections' own prose is not
   * indexed (it is JSX, not text this module holds), so anything a person would
   * plausibly type has to be listed by hand: the row labels, the file names,
   * and the vocabulary the feature goes by elsewhere in the app. Words already
   * in the label or the group's label are not repeated.
   */
  terms: string;
}

/** Rail order, top to bottom. Project sits between the global groups it is
 *  most often confused with, not at the end where it reads as an afterthought. */
export const GROUPS: Group[] = [
  { id: "interface", label: "Interface", scope: "global" },
  { id: "agents", label: "Agents", scope: "global" },
  { id: "project", label: "Project", scope: "project" },
  { id: "app", label: "Application", scope: "global" },
];

export const SECTIONS: Section[] = [
  { id: "appearance", label: "Appearance", group: "interface",
    terms: "theme colour color dark light mode palette contrast accent swatches" },
  { id: "editor", label: "Editor", group: "interface",
    terms: "word wrap file viewer code text lines" },
  { id: "messages", label: "Messages", group: "interface",
    // "hidden" because this section was called Hidden messages until AGE-187,
    // and "ask" because "don't ask again" is the wording on the checkboxes
    // themselves; both are what a person types looking for it.
    terms: "hidden hints tips explanations warnings confirmations dialogs hush silence hide dismiss suppress ask stop showing again" },
  { id: "notifications", label: "Notifications", group: "interface",
    terms: "notify alerts idle seconds banner agent finished exited crashed merge loop watching" },
  { id: "agents", label: "Agent profiles", group: "agents",
    terms: "default model worktree command arguments resume loop environment cli catalog custom" },
  { id: "mcp", label: "MCP servers", group: "agents",
    terms: "model context protocol tools stdio http sse oauth authenticate headers env import json" },
  { id: "localModel", label: "Local model", group: "agents",
    terms: "lm studio openai base url endpoint compatible offline" },
  { id: "backlog", label: "Backlog", group: "project",
    terms: "issues sync share remote git ref team tracker" },
  { id: "knowledge", label: "Knowledge graph", group: "project",
    terms: "graphify index build rebuild stop backend model uv context" },
  { id: "files", label: "Worktree files", group: "project",
    terms: "copy env certs keys secrets untracked service account" },
  { id: "workspace", label: "Workspace", group: "app",
    terms: "journal notes daily weekly planning folder location move switch git hide markdown" },
  { id: "diagnostics", label: "Diagnostics", group: "app",
    // "privacy" and "network" because the launch update check is the one
    // setting in the app those words point at. The agents' own names are not
    // here: this section lists whichever CLIs are on *this* machine, so they
    // arrive as extra terms.
    terms: "version update upgrade release logs report bug github check launch path cli privacy telemetry analytics network npm homebrew brew installer" },
  { id: "about", label: "About", group: "app",
    terms: "licence license apache third party notices source code credits" },
];

export const SECTION_BY_ID = Object.fromEntries(SECTIONS.map((s) => [s.id, s])) as Record<SectionId, Section>;
export const GROUP_BY_ID = Object.fromEntries(GROUPS.map((g) => [g.id, g])) as Record<GroupId, Group>;

export const scopeOf = (id: SectionId): Scope => GROUP_BY_ID[SECTION_BY_ID[id].group].scope;

/**
 * Names only the running app knows: the agent profiles and MCP servers on this
 * machine, and the selected project. Typing "linear", or the project's own
 * name, is how people look for those settings, and none of those words exist
 * until runtime.
 */
export type ExtraTerms = Partial<Record<SectionId, string>>;

/**
 * English connectors, dropped from a query. Only words that carry no meaning
 * for any setting are here: a word that could name one has to keep narrowing.
 */
const STOP_WORDS = new Set([
  "a", "an", "and", "are", "for", "in", "is", "it", "me", "my",
  "of", "on", "or", "the", "to", "with",
  // The imperative half of how people phrase a settings search: "turn off
  // notifications" and "where do I change the theme" both found nothing,
  // because every word has to hit and none of these hits anything. "show",
  // "hide" and "again" are deliberately absent: those are indexed words on the
  // Messages section, where they narrow rather than noise.
  "can", "change", "disable", "do", "does", "don't", "dont", "enable", "how",
  "i", "make", "off", "set", "turn", "what", "where", "which", "you", "your",
]);

/**
 * A typed word with its plural "s" taken off. Substring matching already
 * covers a query shorter than the indexed word ("swatch" finds "swatches"),
 * so only the other direction needs help, and it is the direction people
 * actually type: "themes" and "colors" both matched nothing while "theme" and
 * "color" matched. Left alone at four letters or fewer, where the trailing
 * letter is more likely part of the word than a plural ("sse", "css").
 */
const singular = (w: string): string => (w.length > 4 && w.endsWith("s") ? w.slice(0, -1) : w);

/**
 * The sections a query selects, or null when the box is empty — null means no
 * search is running, which is a different state from a search that matched
 * nothing.
 *
 * Every typed word has to appear somewhere in one section's label, its group's
 * label, its terms or its extra terms. All words, not any: "mcp linear" is a
 * request for the MCP section *because of* linear, and answering it with every
 * section that mentions a model would be no answer at all.
 *
 * When nothing clears that bar, the sections that matched the *most* words win
 * instead. A hand-written term list can never hold every phrasing, and the all-
 * words rule turns each gap into a blank screen: "turn off notifications" and
 * "claude version" both named a section that was right there and got nothing.
 * Answering with the closest sections costs a stop-word list nothing and is
 * never worse than the empty state, which is still what a query that hits no
 * section at all ("kubernetes") returns.
 */
export function matchSections(query: string, extra: ExtraTerms = {}): Set<SectionId> | null {
  const raw = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (raw.length === 0) return null;
  // Dropped before matching, because every word has to hit and these hit
  // nothing: "check for updates" found no section at all, on "for". Falls back
  // to the words as typed when the query is nothing but these, so "the" still
  // searches (it is a prefix of "theme") rather than reading as an empty box.
  const words = raw.filter((w) => !STOP_WORDS.has(w));
  const wanted = words.length > 0 ? words : raw;
  const scored = SECTIONS.map((s) => {
    const hay = `${s.label} ${GROUP_BY_ID[s.group].label} ${s.terms} ${extra[s.id] ?? ""}`.toLowerCase();
    return { id: s.id, hits: wanted.filter((w) => hay.includes(w) || hay.includes(singular(w))).length };
  });
  const best = Math.max(...scored.map((s) => s.hits));
  if (best === 0) return new Set();
  return new Set(scored.filter((s) => s.hits === best).map((s) => s.id));
}
