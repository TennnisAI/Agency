import { useEffect, useMemo, useRef, useState } from "react";
import {
  BackendSearchHit, FileRoot, Issue, Project,
  detectDocsDir, listIssues, listProjects, readDocsCorpus, searchFiles,
} from "../api";
import { useRuns } from "../store/runs";
import { filterEntries } from "../lib/paletteFilter";
import { parsePaletteQuery } from "../lib/paletteQuery";
import { availableCommands } from "../lib/paletteCommands";
import { loadRecency, recencyIndex, recentByPrefix, recordActivation } from "../lib/recency";
import { PENDING_ISSUE_KEY, STATUS_LABELS, isClosed, issueLabel } from "../lib/issues";
import { workspaceHidden } from "../lib/workspacePref";
import { DocsIndex, buildIndex } from "../lib/docsIndex";
import { requestOpenFile } from "../lib/openFile";
import { useListNav } from "../hooks/useListNav";
import { useModalKeys } from "../hooks/useModalKeys";
import { baseName } from "../lib/filePath";

// One palette row, whatever provider it came from. Extends the fuzzy-filter
// entry shape so filterEntries can rank rows directly.
interface Row {
  kind: string;
  id: string;
  projectId: string;
  label: string;
  sublabel: string;
  // Matched but never displayed (issue bodies, dates) — see filterEntries.
  haystack?: string;
  glyph: string;
  section: string;
  activate: () => void;
}

const MAX_ROWS = 50;

/** Split "note:<projectId>:<path>" / "file:<projectId>:<path>" recency keys. */
function parseRecentKey(key: string): { kind: string; projectId: string; path: string } | null {
  const a = key.indexOf(":");
  const b = key.indexOf(":", a + 1);
  if (a < 0 || b < 0) return null;
  return { kind: key.slice(0, a), projectId: key.slice(a + 1, b), path: key.slice(b + 1) };
}

/**
 * The app's front door (one-stop Phase 4): a provider per query mode — plain
 * text over projects/runs/recents/top-commands, ">" commands, "@" issues
 * across every project, "#" tags → notes, "/" file contents. Navigation goes
 * through the Shell handlers (props) so the palette can never drift from the
 * native menu or leave Shell's project state stale.
 */
export default function CommandPalette({
  onClose, onAction, onOpenProject, onOpenRun,
}: {
  onClose: () => void;
  /** Route an onMenu action id (App.tsx routes it like a menu click). */
  onAction: (actionId: string) => void;
  onOpenProject: (p: Project) => void;
  onOpenRun: (p: Project, runId: string) => void;
}) {
  const { runs, selectedProjectId, focusedRunId, setTab } = useRuns();
  const [projects, setProjects] = useState<Project[] | null>(null);
  const [query, setQuery] = useState("");
  useModalKeys(onClose);

  const { mode, term } = parsePaletteQuery(query);

  useEffect(() => {
    listProjects().then(setProjects).catch(() => setProjects([]));
  }, []);

  const selectedProject = projects?.find((p) => p.id === selectedProjectId) ?? null;
  const workspace = workspaceHidden()
    ? null
    : projects?.find((p) => p.kind === "workspace") ?? null;
  // Where "#" and "/" look: the selected project, else the workspace at home.
  const scope = selectedProject ?? workspace;

  // ── "@": every project's issues, fetched once per palette open ────────────
  const [issueData, setIssueData] = useState<{ project: Project; issue: Issue }[] | null>(null);
  const issuesRequested = useRef(false);
  useEffect(() => {
    if (mode !== "issue" || issuesRequested.current || projects === null) return;
    issuesRequested.current = true;
    let cancelled = false;
    Promise.all(
      // Per-project failures drop that project rather than the whole mode.
      projects.map(async (p) => (await listIssues(p.id).catch(() => [])).map((issue) => ({ project: p, issue }))),
    ).then((all) => {
      if (!cancelled) setIssueData(all.flat());
    });
    return () => { cancelled = true; };
  }, [mode, projects]);

  // ── "#": the scope's docs index, built once per palette open ──────────────
  const [docsState, setDocsState] = useState<"idle" | "loading" | "ready" | "none" | "error">("idle");
  const [docsIdx, setDocsIdx] = useState<DocsIndex | null>(null);
  useEffect(() => {
    if (mode !== "tag" || docsState !== "idle" || projects === null) return;
    if (!scope) {
      setDocsState("none");
      return;
    }
    setDocsState("loading");
    let cancelled = false;
    (async () => {
      const dir = await detectDocsDir(scope.id);
      if (dir == null) return "none" as const;
      const files = await readDocsCorpus({ kind: "project", id: scope.id }, dir);
      if (!cancelled) setDocsIdx(buildIndex(files));
      return "ready" as const;
    })()
      .then((s) => { if (!cancelled) setDocsState(s); })
      .catch(() => { if (!cancelled) setDocsState("error"); });
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, docsState, projects, scope?.id]);

  // ── "/": debounced content search over the Files root ─────────────────────
  // Same root the Files tab shows: the focused run's worktree, else the scope
  // project's checkout.
  const contentRoot: FileRoot | null = selectedProject
    ? focusedRunId
      ? { kind: "run", id: focusedRunId }
      : { kind: "project", id: selectedProject.id }
    : workspace
      ? { kind: "project", id: workspace.id }
      : null;
  const [hits, setHits] = useState<BackendSearchHit[] | null>(null);
  const [searchFailed, setSearchFailed] = useState(false);
  const searchToken = useRef(0);
  useEffect(() => {
    if (mode !== "content" || !contentRoot || term.length < 2) {
      setHits(null);
      setSearchFailed(false);
      return;
    }
    const token = ++searchToken.current;
    setHits(null);
    const t = window.setTimeout(() => {
      searchFiles(contentRoot, "", { query: term })
        .then((h) => {
          if (searchToken.current !== token) return;
          setHits(h);
          setSearchFailed(false);
        })
        .catch(() => {
          if (searchToken.current !== token) return;
          setHits([]);
          setSearchFailed(true);
        });
    }, 150);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, term, contentRoot?.kind, contentRoot?.id]);

  // ── activation helpers (each records recency, then routes via Shell) ──────
  function openNote(project: Project, path: string, title: string) {
    recordActivation(`note:${project.id}:${path}`, title, path);
    try { localStorage.setItem(`docs:last:${project.id}`, path); } catch { /* storage unavailable */ }
    onOpenProject(project);
    setTab("docs");
    window.dispatchEvent(new CustomEvent("agency:open-note", { detail: { projectId: project.id, path } }));
    onClose();
  }

  function openFile(project: Project, path: string, line?: number) {
    recordActivation(`file:${project.id}:${path}`, baseName(path), path);
    // Re-selecting the current project would clear run focus (and with it the
    // worktree root "/" just searched) — only switch when actually elsewhere.
    if (selectedProjectId !== project.id) onOpenProject(project);
    setTab("files");
    requestOpenFile({ path, line });
    onClose();
  }

  function openIssue(project: Project, issue: Issue) {
    recordActivation(`issue:${issue.id}`, `${issueLabel(project, issue)} ${issue.title}`, project.name);
    sessionStorage.setItem(PENDING_ISSUE_KEY, issue.id);
    onOpenProject(project);
    setTab("issues");
    onClose();
  }

  const ctx = {
    hasProject: !!selectedProjectId,
    hasFocusedAgent: runs.some((r) => r.id === focusedRunId && r.kind === "agent"),
    workspaceVisible: !workspaceHidden(),
  };
  const commandRow = (id: string, label: string, sublabel: string): Row => ({
    kind: "command", id, projectId: "", label, sublabel, glyph: "›", section: "Commands",
    activate: () => {
      recordActivation(`cmd:${id}`, label);
      onAction(id);
      onClose();
    },
  });

  // ── rows per mode ─────────────────────────────────────────────────────────
  const rows = useMemo<Row[]>(() => {
    if (mode === "command") {
      const cmds = availableCommands(ctx).map((c) => commandRow(c.id, c.label, c.sublabel));
      return filterEntries(term, cmds).slice(0, MAX_ROWS);
    }

    if (mode === "issue") {
      if (issueData === null) return [];
      // Selected project first, open work before closed; filterEntries keeps
      // this order within a match tier.
      const entries = issueData.map(({ project, issue }) => ({
        rank: (project.id === selectedProjectId ? 0 : 2) + (isClosed(issue.status) ? 1 : 0),
        row: {
          kind: "issue",
          id: issue.id,
          projectId: project.id,
          label: `${issueLabel(project, issue)} ${issue.title}`,
          sublabel: `${project.name} · ${STATUS_LABELS[issue.status]}`,
          // Body text and dates are searchable without being shown.
          haystack: [issue.body, issue.due, issue.scheduled].filter(Boolean).join(" "),
          glyph: "▧",
          section: "Issues",
          activate: () => openIssue(project, issue),
        } satisfies Row,
      }));
      entries.sort((a, b) => a.rank - b.rank);
      return filterEntries(term, entries.map((e) => e.row)).slice(0, MAX_ROWS);
    }

    if (mode === "tag") {
      if (!docsIdx || docsState !== "ready") return [];
      const tags = [...docsIdx.tags.entries()];
      if (!term) {
        // Bare "#": browse tags; activating one narrows the query instead of
        // closing, so the flow is tag → its notes.
        tags.sort((a, b) => b[1].length - a[1].length || a[0].localeCompare(b[0]));
        return tags.slice(0, MAX_ROWS).map(([tag, paths]): Row => ({
          kind: "tag", id: tag, projectId: "", label: `#${tag}`,
          sublabel: `${paths.length} note${paths.length === 1 ? "" : "s"}`,
          glyph: "#", section: "Tags",
          activate: () => setQuery(`#${tag}`),
        }));
      }
      const t = term.toLowerCase();
      const seen = new Set<string>();
      const out: Row[] = [];
      for (const [tag, paths] of tags) {
        if (!tag.startsWith(t)) continue;
        for (const path of paths) {
          if (seen.has(path)) continue;
          seen.add(path);
          const doc = docsIdx.docs.get(path);
          out.push({
            kind: "note", id: path, projectId: scope?.id ?? "",
            label: doc?.title ?? path, sublabel: `#${tag} · ${path}`,
            glyph: "▥", section: "Notes",
            activate: () => { if (scope) openNote(scope, path, doc?.title ?? path); },
          });
          if (out.length >= MAX_ROWS) return out;
        }
      }
      return out;
    }

    if (mode === "content") {
      if (!hits || !scope) return [];
      return hits.slice(0, MAX_ROWS).map((h): Row => ({
        kind: "hit", id: `${h.path}:${h.line}`, projectId: scope.id,
        label: h.text.trim(), sublabel: `${h.path}:${h.line}`,
        glyph: "▤", section: "Matches",
        activate: () => openFile(scope, h.path, h.line),
      }));
    }

    // Default mode: top commands, recent notes/files, projects, runs — recency
    // pre-sorts the pool so recent entries win their match tier.
    if (projects === null) return [];
    const cmds = availableCommands(ctx);
    const staples = cmds.filter((c) => c.id === "daily-note" || c.id === "new-issue");
    const recentCmdIds = new Set(recentByPrefix("cmd:", 10).map((e) => e.key.slice(4)));
    const topCmds = [
      ...staples,
      ...cmds.filter((c) => recentCmdIds.has(c.id) && !staples.includes(c)).slice(0, 3),
    ].map((c) => commandRow(c.id, c.label, c.sublabel));

    const byId = new Map(projects.map((p) => [p.id, p]));
    const recents: Row[] = loadRecency()
      .filter((e) => e.key.startsWith("note:") || e.key.startsWith("file:"))
      .slice(0, 8)
      .flatMap((e) => {
        const parsed = parseRecentKey(e.key);
        const project = parsed && byId.get(parsed.projectId);
        if (!parsed || !project) return []; // project gone — drop the recent
        const note = parsed.kind === "note";
        return [{
          kind: note ? "note" : "file", id: e.key, projectId: project.id,
          label: e.label, sublabel: `${project.name} · ${e.sub}`,
          glyph: note ? "▥" : "▤", section: "Recent",
          activate: () => (note
            ? openNote(project, parsed.path, e.label)
            : openFile(project, parsed.path)),
        }];
      });

    const projectRows = projects
      .filter((p) => p.kind !== "workspace" || !workspaceHidden())
      .map((p): Row => ({
        kind: "project", id: p.id, projectId: p.id, label: p.name, sublabel: p.repo_path,
        glyph: p.kind === "workspace" ? "◈" : "▢", section: "Projects",
        activate: () => {
          recordActivation(`project:${p.id}`, p.name, p.repo_path);
          onOpenProject(p);
          onClose();
        },
      }));

    const runRows = runs.map((r): Row => ({
      kind: "run", id: r.id, projectId: r.projectId,
      label: `${r.agent}: ${r.title || r.prompt || r.branch}`, sublabel: r.branch,
      glyph: "▸", section: "Agents",
      activate: () => {
        const p = byId.get(r.projectId);
        if (!p) return;
        recordActivation(`run:${r.id}`, `${r.agent}: ${r.title || r.prompt || r.branch}`, r.branch);
        onOpenRun(p, r.id);
        onClose();
      },
    }));

    const pool = [...topCmds, ...recents, ...projectRows, ...runRows];
    if (!term) return pool.slice(0, MAX_ROWS);
    const rec = recencyIndex();
    const recKey = (r: Row) => (r.kind === "command" ? `cmd:${r.id}`
      : r.kind === "project" ? `project:${r.id}`
      : r.kind === "run" ? `run:${r.id}` : r.id);
    const ranked = pool
      .map((r, i) => ({ r, i, rec: rec.get(recKey(r)) ?? Infinity }))
      .sort((a, b) => (a.rec - b.rec) || (a.i - b.i))
      .map((x) => x.r);
    return filterEntries(term, ranked).slice(0, MAX_ROWS);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode, term, projects, runs, selectedProjectId, focusedRunId, issueData, docsIdx, docsState, hits]);

  const { hi, setHi, onKey } = useListNav(rows.length, (i) => rows[i].activate(), query);

  // Provider-specific empty/progress states (rendered in place of rows).
  const empty = (() => {
    if (rows.length > 0) return null;
    if (projects === null) return "loading…";
    switch (mode) {
      case "command":
        return "No matching commands";
      case "issue":
        return issueData === null ? "loading…" : "No matching issues";
      case "tag":
        if (docsState === "loading" || docsState === "idle") return "loading…";
        if (docsState === "none") return scope ? "No docs folder here" : "Select a project to browse tags";
        if (docsState === "error") return "Couldn't read docs";
        return term ? "No tagged notes" : "No tags";
      case "content":
        if (!contentRoot) return "Select a project to search file contents";
        if (term.length < 2) return "Type at least 2 characters to search";
        if (searchFailed) return "Search failed";
        return hits === null ? "searching…" : "No matches";
      default:
        return "No matches";
    }
  })();

  return (
    <div className="palette-overlay" onClick={onClose}>
      <div className="palette" onClick={(e) => e.stopPropagation()}>
        <input
          className="palette-input"
          autoFocus
          placeholder="Search, or type > @ # / for commands, issues, tags, contents…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={onKey}
        />
        <ul className="palette-list">
          {empty && <li className="palette-empty">{empty}</li>}
          {rows.map((r, i) => (
            <li key={`${r.kind}:${r.id}`}>
              {(i === 0 || rows[i - 1].section !== r.section) && mode === "default" && !term && (
                <div className="palette-section">{r.section}</div>
              )}
              <div
                className={`palette-row ${i === hi ? "on" : ""}`}
                onMouseEnter={() => setHi(i)}
                onClick={() => r.activate()}
              >
                <span className={`palette-kind ${r.kind}`}>{r.glyph}</span>
                <span className="palette-label">{r.label}</span>
                <span className="palette-sub">{r.sublabel}</span>
              </div>
            </li>
          ))}
        </ul>
        {!query && (
          <div className="palette-hint">{"> commands · @ issues · # tags · / file contents"}</div>
        )}
      </div>
    </div>
  );
}
