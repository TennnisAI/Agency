import { useCallback, useEffect, useRef, useState } from "react";
import { DocFile, FileRoot, createDir, detectDocsDir, docsCorpusStats, readDocsFiles } from "../api";
import { DocsIndex, buildIndex } from "../lib/docsIndex";
import { toastError } from "../lib/toast";

/**
 * The Docs tab's data source: detects the project's docs folder (case-
 * insensitive; the workspace's whole folder), loads the markdown corpus, and
 * keeps the index fresh by polling while the tab is active.
 *
 * Polls are cheap by design (one-stop Phase 2): each tick fetches stat
 * signatures only, and bodies are re-read just for files whose mtime/size
 * changed — an idle 500-note corpus costs one stat pass per tick, zero reads,
 * zero index rebuilds. Mutations call `refresh` for instant feedback; the poll
 * picks up external edits (agents writing docs).
 *
 * `docsDir` is undefined while detecting, null when the project has none.
 */
export function useDocs(projectId: string | null, active: boolean) {
  const [docsDir, setDocsDir] = useState<string | null | undefined>(undefined);
  // Tagged with the project it was built from: the reset effect below can only
  // clear it one commit late, and consumers that key work off the index (tab
  // restore, the tree) must never see the outgoing project's corpus.
  const [indexed, setIndexed] = useState<{ pid: string; index: DocsIndex } | null>(null);
  const index = indexed !== null && indexed.pid === projectId ? indexed.index : null;
  const projectRef = useRef(projectId);
  projectRef.current = projectId;
  const dirRef = useRef(docsDir);
  dirRef.current = docsDir;
  // The corpus cache the stat diff runs against: bodies by path + the
  // signature each body was read at. `built` distinguishes "no changes" from
  // "never loaded" so the first pass always builds an index.
  const filesRef = useRef<Map<string, DocFile>>(new Map());
  const sigRef = useRef<Map<string, string>>(new Map());
  // Every folder: the scan reports these because the corpus cannot, and a poll
  // that finds the list changed rebuilds the index even when no note did
  // (creating a folder touches no markdown).
  const dirsRef = useRef<string[]>([]);
  const builtRef = useRef(false);
  // Last pass wins. A mutation refreshes while a poll tick is already in
  // flight, and the tick's older answer (taken before the write) would
  // otherwise land on top and un-create what the user just made until the next
  // tick corrected it.
  const seqRef = useRef(0);

  const refresh = useCallback(async () => {
    const pid = projectRef.current;
    if (!pid) return;
    try {
      let dir = dirRef.current;
      // Re-detect while absent so a docs folder created outside the app (or by
      // an agent) is picked up without leaving the tab.
      if (dir == null) {
        dir = await detectDocsDir(pid);
        if (projectRef.current !== pid) return;
        dirRef.current = dir; // update eagerly — the state ref lags a render
        setDocsDir(dir);
        if (dir == null) return;
      }
      const root: FileRoot = { kind: "project", id: pid };
      const seq = ++seqRef.current;
      const scan = await docsCorpusStats(root, dir);
      if (projectRef.current !== pid || dirRef.current !== dir || seqRef.current !== seq) return;

      const nextSig = new Map(scan.files.map((s) => [s.path, `${s.mtimeMs}:${s.size}`]));
      const changed: string[] = [];
      for (const [path, sig] of nextSig) {
        if (sigRef.current.get(path) !== sig) changed.push(path);
      }
      const removed = [...sigRef.current.keys()].filter((p) => !nextSig.has(p));
      // Both lists arrive sorted, so position-wise comparison is enough.
      const dirsChanged =
        scan.dirs.length !== dirsRef.current.length ||
        scan.dirs.some((d, i) => d !== dirsRef.current[i]);
      if (changed.length === 0 && removed.length === 0 && !dirsChanged && builtRef.current) return;

      if (changed.length > 0) {
        const files = await readDocsFiles(root, dir, changed);
        if (projectRef.current !== pid || dirRef.current !== dir || seqRef.current !== seq) return;
        for (const f of files) filesRef.current.set(f.path, f);
        // A changed path the read skipped (binary, vanished mid-poll) must not
        // linger with its stale body; the signature still records the attempt
        // so it isn't re-fetched every tick.
        const returned = new Set(files.map((f) => f.path));
        for (const p of changed) {
          if (!returned.has(p)) filesRef.current.delete(p);
        }
      }
      for (const p of removed) filesRef.current.delete(p);
      sigRef.current = nextSig;
      dirsRef.current = scan.dirs;
      builtRef.current = true;
      if (import.meta.env.DEV && (changed.length > 0 || removed.length > 0)) {
        // Ship-gate probe: an idle corpus logs nothing.
        console.debug(`[docs] corpus refresh: ${changed.length} read, ${removed.length} removed`);
      }
      setIndexed({ pid, index: buildIndex([...filesRef.current.values()], scan.dirs) });
    } catch {
      /* transient IPC errors: keep the last good index */
    }
  }, []);

  useEffect(() => {
    setDocsDir(undefined);
    setIndexed(null);
    filesRef.current = new Map();
    sigRef.current = new Map();
    dirsRef.current = [];
    builtRef.current = false;
    if (!active || !projectId) return;
    void refresh();
    const t = window.setInterval(refresh, 2000);
    return () => window.clearInterval(t);
  }, [projectId, active, refresh]);

  const createDocsDir = useCallback(async () => {
    const pid = projectRef.current;
    if (!pid) return;
    try {
      await createDir({ kind: "project", id: pid }, "docs");
      dirRef.current = "docs";
      setDocsDir("docs");
      await refresh();
    } catch (e) {
      toastError(e, "Couldn't create docs folder");
    }
  }, [refresh]);

  return { docsDir, index, refresh, createDocsDir };
}
