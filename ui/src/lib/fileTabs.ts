// Pure tab-state transitions for the Files editor (one-stop Phase 3), reused by
// the Docs tab. The components own rendering and persistence; every rule that
// can be unit-tested lives here.

export interface TabState {
  /** Strip order = open order. */
  open: string[];
  active: string | null;
  /** Activation order, most-recent-last. Cold (restored) tabs are absent. */
  recency: string[];
  dirty: Set<string>;
}

/** Live CodeMirror instances are expensive; evict clean tabs past this. */
export const MAX_TABS = 10;

export const emptyTabs = (): TabState => ({ open: [], active: null, recency: [], dirty: new Set() });

const bumpRecency = (recency: string[], path: string): string[] =>
  [...recency.filter((p) => p !== path), path];

/** Open (appending + possibly evicting) or just activate an existing tab. */
export function openTab(s: TabState, path: string): TabState {
  if (s.open.includes(path)) return activateTab(s, path);
  let open = [...s.open, path];
  if (open.length > MAX_TABS) {
    // LRU candidates: clean, not the (old) active, never-activated first.
    const cold = s.open.filter((p) => !s.recency.includes(p));
    const victim = [...cold, ...s.recency].find((p) => !s.dirty.has(p) && p !== s.active);
    if (victim !== undefined) open = open.filter((p) => p !== victim);
  }
  return {
    open,
    active: path,
    recency: bumpRecency(s.recency.filter((p) => open.includes(p)), path),
    dirty: s.dirty,
  };
}

export function activateTab(s: TabState, path: string): TabState {
  if (!s.open.includes(path)) return s;
  return { ...s, active: path, recency: bumpRecency(s.recency, path) };
}

/** Close one tab; the active slot moves to the nearest right, else left. */
export function closeTab(s: TabState, path: string): TabState {
  const idx = s.open.indexOf(path);
  if (idx < 0) return s;
  const open = s.open.filter((p) => p !== path);
  const dirty = new Set(s.dirty);
  dirty.delete(path);
  const active = s.active === path ? open[idx] ?? open[idx - 1] ?? null : s.active;
  return {
    open,
    active,
    recency: s.recency.filter((p) => p !== path),
    dirty,
  };
}

export function setDirtyTab(s: TabState, path: string, isDirty: boolean): TabState {
  if (s.dirty.has(path) === isDirty) return s;
  const dirty = new Set(s.dirty);
  if (isDirty) dirty.add(path);
  else dirty.delete(path);
  return { ...s, dirty };
}

/** `from` → `to` for the path itself or anything under it (dir rename). */
export const retargetPath = (p: string, from: string, to: string): string =>
  p === from ? to : p.startsWith(from + "/") ? to + p.slice(from.length) : p;
const retarget = retargetPath;

/** A file or directory was renamed on disk; follow it everywhere. */
export function renameTab(s: TabState, from: string, to: string): TabState {
  return {
    open: s.open.map((p) => retarget(p, from, to)),
    active: s.active === null ? null : retarget(s.active, from, to),
    recency: s.recency.map((p) => retarget(p, from, to)),
    dirty: new Set([...s.dirty].map((p) => retarget(p, from, to))),
  };
}

/** A file or directory was deleted; close its tab(s). */
export function removeTab(s: TabState, path: string): TabState {
  return s.open
    .filter((p) => p === path || p.startsWith(path + "/"))
    .reduce((acc, p) => closeTab(acc, p), s);
}

export const serializeTabs = (s: TabState): string =>
  JSON.stringify({ open: s.open, active: s.active });

/** Paths only — restored tabs are cold and clean until first activation. */
export function deserializeTabs(raw: string | null): TabState {
  if (!raw) return emptyTabs();
  try {
    const v = JSON.parse(raw) as { open?: unknown; active?: unknown };
    const open = Array.isArray(v.open) ? v.open.filter((p): p is string => typeof p === "string") : [];
    const active = typeof v.active === "string" && open.includes(v.active) ? v.active : null;
    return { open, active, recency: [], dirty: new Set() };
  } catch {
    return emptyTabs();
  }
}
