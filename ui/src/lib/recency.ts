// Palette recency (one-stop Phase 4): the last-N activations, persisted so
// ranking and the "recent notes/files" section survive restarts. Keys are
// namespaced — cmd:<id>, project:<id>, run:<id>, issue:<id>,
// note:<projectId>:<path>, file:<projectId>:<path> — and each entry keeps the
// label it was shown with so recents can render without refetching.

export interface RecencyEntry {
  key: string;
  label: string;
  sub: string;
  t: number;
}

const STORE_KEY = "palette:recency:v1";
const CAP = 100;

type Store = Pick<Storage, "getItem" | "setItem">;

function defaultStore(): Store | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null; // storage disabled — recency silently off
  }
}

function isEntry(e: unknown): e is RecencyEntry {
  const r = e as RecencyEntry;
  return !!r && typeof r.key === "string" && typeof r.label === "string"
    && typeof r.sub === "string" && typeof r.t === "number";
}

export function loadRecency(storage: Store | null = defaultStore()): RecencyEntry[] {
  if (!storage) return [];
  try {
    const raw = storage.getItem(STORE_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? parsed.filter(isEntry) : [];
  } catch {
    return [];
  }
}

/** Record an activation: dedupe by key, newest first, capped. */
export function recordActivation(
  key: string,
  label: string,
  sub = "",
  storage: Store | null = defaultStore(),
  now = Date.now(),
): void {
  if (!storage) return;
  const list = loadRecency(storage).filter((e) => e.key !== key);
  list.unshift({ key, label, sub, t: now });
  try {
    storage.setItem(STORE_KEY, JSON.stringify(list.slice(0, CAP)));
  } catch {
    /* quota/unavailable — recency is best-effort */
  }
}

/** Newest-first entries whose key starts with `prefix`, capped at `n`. */
export function recentByPrefix(prefix: string, n: number, storage: Store | null = defaultStore()): RecencyEntry[] {
  return loadRecency(storage).filter((e) => e.key.startsWith(prefix)).slice(0, n);
}

/** key → recency rank (0 = most recent) for ranking boosts. */
export function recencyIndex(storage: Store | null = defaultStore()): Map<string, number> {
  const m = new Map<string, number>();
  loadRecency(storage).forEach((e, i) => m.set(e.key, i));
  return m;
}
