import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  buildKnowledgeGraph,
  getKnowledgeConfig,
  KnowledgeConfig,
  KnowledgeGraphView,
  knowledgeGraphView,
  MapFile,
  Project,
} from "../api";
import { fileRootKey, requestOpenFile } from "../lib/openFile";
import {
  edgePath,
  edgeSummary,
  findDir,
  findFile,
  layout,
  LevelEdge,
  levelEdges,
  LevelNode,
  levelNodes,
  parentPath,
  symbolEdgeIndex,
  symbolsById,
  viewBox,
} from "../lib/maplayout";
import { toastError, toastInfo } from "../lib/toast";
import { useRuns } from "../store/runs";

// The Map tab: the project's knowledge graph as a drill-down dependency map.
// Boxes are subdirectories and files of the current level, lines are the
// dependencies between them; clicking a directory descends into it and
// clicking a file opens its symbols and dependencies in the side panel, down
// to the individual function. The data is graphify's graph.json, reduced
// backend-side (graphview.rs) and fetched in one call.
export default function MapView({ project }: { project: Project }) {
  const { runs, selectedRunId, setView, setTab } = useRuns();
  const [data, setData] = useState<KnowledgeGraphView | null>(null);
  // Loaded when the view is unavailable, to say why and offer the fix.
  const [kg, setKg] = useState<KnowledgeConfig | null>(null);
  const [loading, setLoading] = useState(true);
  const [path, setPath] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [hover, setHover] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      const view = await knowledgeGraphView(project.id);
      setData(view);
      setKg(null);
    } catch {
      // No graph to show; the config says which empty state this is.
      setData(null);
      try {
        setKg(await getKnowledgeConfig(project.id));
      } catch (e) {
        toastError(e, "knowledge graph");
      }
    } finally {
      setLoading(false);
    }
  }, [project.id]);

  useEffect(() => {
    setData(null);
    setKg(null);
    setLoading(true);
    setPath("");
    setSelected(null);
    void reload();
  }, [reload]);

  // A build in flight (started here or in Settings) resolves into either a
  // fresh view or a build error, so poll the config until it settles.
  const [building, setBuilding] = useState(false);
  useEffect(() => {
    if (!building) return;
    const t = window.setInterval(async () => {
      try {
        const cfg = await getKnowledgeConfig(project.id);
        if (!cfg.building) {
          setBuilding(false);
          setKg(cfg);
          void reload();
        }
      } catch {
        setBuilding(false);
      }
    }, 2000);
    return () => window.clearInterval(t);
  }, [building, project.id, reload]);

  const startBuild = async () => {
    try {
      await buildKnowledgeGraph(project.id);
      setBuilding(true);
    } catch (e) {
      toastError(e, "graph build");
    }
  };

  // Open a file (optionally at a line) in the Files tab. Files follows the
  // selected run, so a selected worktree run is dropped back to the grid
  // first: the map describes the project checkout, not an agent's branch.
  const openInFiles = (filePath: string, line?: number) => {
    const run = runs.find((r) => r.id === selectedRunId);
    const root =
      run && !run.worktree
        ? { kind: "run", id: run.id } // same tree as the checkout
        : { kind: "project", id: project.id };
    if (run?.worktree) setView("grid");
    setTab("files");
    requestOpenFile({ rootKey: fileRootKey(root), path: filePath, line, reveal: true });
  };

  // Jump the map to a file that may sit anywhere in the tree.
  const revealFile = (filePath: string) => {
    setPath(parentPath(filePath));
    setSelected(filePath);
  };

  const dir = useMemo(
    () => (data ? (findDir(data.root, path) ?? data.root) : null),
    [data, path],
  );
  const level = useMemo(() => {
    if (!data || !dir) return null;
    const nodes = levelNodes(dir);
    const edges = levelEdges(data.file_edges, nodes);
    return { nodes: layout(nodes, edges), edges };
  }, [data, dir]);

  const symbolIndex = useMemo(() => (data ? symbolEdgeIndex(data.symbol_edges) : null), [data]);
  const allSymbols = useMemo(() => (data ? symbolsById(data.root) : null), [data]);

  if (loading) return <div className="board empty">Loading the map…</div>;

  if (!data) {
    const state = building
      ? "building"
      : !kg
        ? "missing"
        : !kg.graph
          ? "off"
          : kg.building
            ? "building"
            : kg.last_build_error
              ? "failed"
              : !kg.build_installed
                ? "tooling"
                : "unbuilt";
    return (
      <div className="board empty">
        <div className="map-empty">
          {state === "off" && (
            <p>
              The map is drawn from this project's knowledge graph, which is turned off.
              Enable it in Settings ▸ Knowledge graph.
            </p>
          )}
          {state === "tooling" && (
            <p>
              The knowledge graph is enabled but its build tooling is not installed.
              Install it from Settings ▸ Knowledge graph.
            </p>
          )}
          {state === "unbuilt" && (
            <>
              <p>No graph has been built for this project yet.</p>
              <button className="map-btn" onClick={() => void startBuild()}>Build graph</button>
            </>
          )}
          {state === "building" && <p>Building the graph…</p>}
          {state === "failed" && (
            <>
              <p>The last graph build failed: {kg?.last_build_error}</p>
              <button className="map-btn" onClick={() => void startBuild()}>Try again</button>
            </>
          )}
          {state === "missing" && <p>The map is unavailable for this project.</p>}
        </div>
      </div>
    );
  }

  const crumbs = path === "" ? [] : path.split("/");
  const selectedFile = selected && data ? findFile(data.root, selected) : null;
  const stats = data.stats;

  return (
    <div className="map">
      <div className="map-head">
        <div className="map-crumbs">
          <button
            className={path === "" ? "on" : ""}
            onClick={() => {
              setPath("");
              setSelected(null);
            }}
          >
            {project.name}
          </button>
          {crumbs.map((part, i) => (
            <span key={i}>
              <span className="map-crumb-sep" aria-hidden>▸</span>
              <button
                className={i === crumbs.length - 1 ? "on" : ""}
                onClick={() => {
                  setPath(crumbs.slice(0, i + 1).join("/"));
                  setSelected(null);
                }}
              >
                {part}
              </button>
            </span>
          ))}
        </div>
        <div className="spacer" />
        <span className="map-stats">
          {stats.files} files · {stats.symbols} symbols · {stats.edges} links
        </span>
        <button
          className="icon-btn"
          title={building ? "A graph build is running" : "Rebuild the graph"}
          disabled={building}
          onClick={() => void startBuild()}
        >
          {building ? "…" : "↻"}
        </button>
      </div>
      <div className="map-body">
        {level && (
          <MapCanvas
            nodes={level.nodes}
            edges={level.edges}
            selected={selected}
            hover={hover}
            onHover={setHover}
            onPick={(node) => {
              if (node.kind === "dir") {
                setPath(node.key);
                setSelected(null);
              } else {
                setSelected(node.key === selected ? null : node.key);
              }
            }}
            onBackground={() => setSelected(null)}
            onAscend={() => {
              if (path !== "") {
                setPath(parentPath(path));
                setSelected(null);
              }
            }}
          />
        )}
        <div className="map-side">
          {selectedFile && symbolIndex && allSymbols ? (
            <FilePanel
              file={selectedFile}
              data={data}
              symbolIndex={symbolIndex}
              allSymbols={allSymbols}
              onOpen={openInFiles}
              onReveal={revealFile}
            />
          ) : (
            <LevelPanel dirPath={path} data={data} nodes={level?.nodes ?? []} onReveal={revealFile} onSelect={setSelected} />
          )}
        </div>
      </div>
    </div>
  );
}

// The drawing surface: laid-out boxes and dependency lines, with wheel zoom
// and drag pan. Pure props in, events out — the harness renders it without
// the app behind it.
export function MapCanvas({
  nodes,
  edges,
  selected,
  hover,
  onHover,
  onPick,
  onBackground,
  onAscend,
}: {
  nodes: LevelNode[];
  edges: LevelEdge[];
  selected: string | null;
  hover: string | null;
  onHover: (key: string | null) => void;
  onPick: (node: LevelNode) => void;
  onBackground: () => void;
  onAscend: () => void;
}) {
  const [cam, setCam] = useState({ scale: 1, tx: 0, ty: 0 });
  const svgRef = useRef<SVGSVGElement | null>(null);
  const drag = useRef<{ x: number; y: number; tx: number; ty: number; moved: boolean } | null>(null);
  const vb = useMemo(() => viewBox(nodes), [nodes]);
  useEffect(() => setCam({ scale: 1, tx: 0, ty: 0 }), [vb]);

  const byKey = useMemo(() => new Map(nodes.map((n) => [n.key, n])), [nodes]);

  // Client pixel → world coordinate, through the viewBox and the camera.
  const toWorld = (clientX: number, clientY: number) => {
    const svg = svgRef.current!;
    const rect = svg.getBoundingClientRect();
    const [x0, y0, w, h] = vb.split(" ").map(Number);
    // preserveAspectRatio="xMidYMid meet": one uniform scale, centered.
    const fit = Math.min(rect.width / w, rect.height / h);
    const px = x0 + w / 2 + (clientX - rect.left - rect.width / 2) / fit;
    const py = y0 + h / 2 + (clientY - rect.top - rect.height / 2) / fit;
    return { x: (px - cam.tx) / cam.scale, y: (py - cam.ty) / cam.scale };
  };

  const onWheel = (e: React.WheelEvent) => {
    const factor = Math.exp(-e.deltaY * 0.0015);
    const next = Math.min(3, Math.max(0.25, cam.scale * factor));
    if (next === cam.scale) return;
    const at = toWorld(e.clientX, e.clientY);
    // Keep the point under the cursor fixed while the scale changes.
    setCam({
      scale: next,
      tx: at.x * (cam.scale - next) + cam.tx,
      ty: at.y * (cam.scale - next) + cam.ty,
    });
  };

  const connected = (key: string): boolean =>
    edges.some(
      (e) =>
        (e.source === key || e.target === key) &&
        (e.source === hover || e.target === hover || e.source === selected || e.target === selected),
    );
  const focusKey = hover ?? selected;

  return (
    <svg
      ref={svgRef}
      className="map-canvas"
      viewBox={vb}
      preserveAspectRatio="xMidYMid meet"
      onWheel={onWheel}
      onPointerDown={(e) => {
        (e.target as Element).setPointerCapture?.(e.pointerId);
        drag.current = { x: e.clientX, y: e.clientY, tx: cam.tx, ty: cam.ty, moved: false };
      }}
      onPointerMove={(e) => {
        if (!drag.current) return;
        const svg = svgRef.current!;
        const rect = svg.getBoundingClientRect();
        const [, , w, h] = vb.split(" ").map(Number);
        const fit = Math.min(rect.width / w, rect.height / h);
        const dx = (e.clientX - drag.current.x) / fit;
        const dy = (e.clientY - drag.current.y) / fit;
        if (Math.abs(e.clientX - drag.current.x) + Math.abs(e.clientY - drag.current.y) > 3)
          drag.current.moved = true;
        setCam((c) => ({ ...c, tx: drag.current!.tx + dx, ty: drag.current!.ty + dy }));
      }}
      onPointerUp={(e) => {
        const wasDrag = drag.current?.moved;
        drag.current = null;
        if (!wasDrag && e.target === svgRef.current) onBackground();
      }}
      onDoubleClick={(e) => {
        if (e.target === svgRef.current) onAscend();
      }}
    >
      <g transform={`translate(${cam.tx} ${cam.ty}) scale(${cam.scale})`}>
        {edges.map((e) => {
          const a = byKey.get(e.source);
          const b = byKey.get(e.target);
          if (!a || !b) return null;
          const { d, arrow } = edgePath(a, b);
          const on = focusKey !== null && (e.source === focusKey || e.target === focusKey);
          const dim = focusKey !== null && !on;
          const width = Math.min(4, 1 + Math.log2(e.weight + 1) * 0.5);
          return (
            <g key={`${e.source} ${e.target}`} className={`map-edge${on ? " on" : ""}${dim ? " dim" : ""}`}>
              <path d={d} fill="none" strokeWidth={width}>
                <title>{`${e.source} → ${e.target}: ${edgeSummary(e)}`}</title>
              </path>
              <polygon points={arrow} stroke="none" />
            </g>
          );
        })}
        {nodes.map((n) => {
          const on = n.key === focusKey || n.key === selected;
          const dim = focusKey !== null && !on && !connected(n.key);
          const maxChars = Math.floor((n.w - 18) / 6.6);
          const name = n.kind === "dir" ? n.name + "/" : n.name;
          const label = name.length > maxChars ? name.slice(0, maxChars - 1) + "…" : name;
          const meta =
            n.kind === "dir"
              ? `${n.files} ${n.files === 1 ? "file" : "files"} · ${n.symbols} sym`
              : `${n.symbols} ${n.symbols === 1 ? "symbol" : "symbols"}`;
          return (
            <g
              key={n.key}
              className={`map-node ${n.kind}${on ? " on" : ""}${dim ? " dim" : ""}`}
              transform={`translate(${n.x - n.w / 2} ${n.y - n.h / 2})`}
              onPointerEnter={() => onHover(n.key)}
              onPointerLeave={() => onHover(null)}
              onPointerUp={(e) => {
                if (!drag.current?.moved) {
                  e.stopPropagation();
                  drag.current = null;
                  onPick(n);
                }
              }}
            >
              <rect width={n.w} height={n.h} rx={6} />
              <text className="map-node-name" x={10} y={17}>{label}</text>
              <text className="map-node-meta" x={10} y={31}>{meta}</text>
              <title>{n.key}</title>
            </g>
          );
        })}
      </g>
    </svg>
  );
}

// Side panel with nothing selected: what this level contains, biggest files
// first, so there is always somewhere obvious to go next.
function LevelPanel({
  dirPath,
  data,
  nodes,
  onReveal,
  onSelect,
}: {
  dirPath: string;
  data: KnowledgeGraphView;
  nodes: LevelNode[];
  onReveal: (path: string) => void;
  onSelect: (path: string) => void;
}) {
  const files = nodes.filter((n) => n.kind === "file").sort((a, b) => b.symbols - a.symbols);
  // The most-connected files under this level, a "start here" list.
  const hubs = useMemo(() => {
    const weight = new Map<string, number>();
    const inside = (p: string) => dirPath === "" || p === dirPath || p.startsWith(dirPath + "/");
    for (const e of data.file_edges) {
      const w = e.calls + e.imports + e.refs + e.other;
      if (inside(e.source)) weight.set(e.source, (weight.get(e.source) ?? 0) + w);
      if (inside(e.target)) weight.set(e.target, (weight.get(e.target) ?? 0) + w);
    }
    return [...weight.entries()].sort((a, b) => b[1] - a[1]).slice(0, 8);
  }, [data, dirPath]);
  return (
    <>
      <div className="map-side-title">{dirPath === "" ? "Project" : dirPath}</div>
      <div className="map-side-hint">
        Click a directory to descend into it, a file for its symbols and
        dependencies. Scroll to zoom, drag to pan, double-click the background
        to go back up.
      </div>
      {hubs.length > 0 && (
        <>
          <div className="map-side-sec">Most connected</div>
          {hubs.map(([p, w]) => (
            <button key={p} className="map-row" onClick={() => onReveal(p)}>
              <span className="map-row-label">{p}</span>
              <span className="map-row-meta">{w}</span>
            </button>
          ))}
        </>
      )}
      {files.length > 0 && (
        <>
          <div className="map-side-sec">Files here</div>
          {files.map((f) => (
            <button key={f.key} className="map-row" onClick={() => onSelect(f.key)}>
              <span className="map-row-label">{f.name}</span>
              <span className="map-row-meta">{f.symbols}</span>
            </button>
          ))}
        </>
      )}
    </>
  );
}

// Side panel for a selected file: open it, walk its symbols (expandable down
// to per-symbol edges), and follow its dependencies in either direction.
function FilePanel({
  file,
  data,
  symbolIndex,
  allSymbols,
  onOpen,
  onReveal,
}: {
  file: MapFile;
  data: KnowledgeGraphView;
  symbolIndex: ReturnType<typeof symbolEdgeIndex>;
  allSymbols: Map<string, { label: string; line: number | null; file: string }>;
  onOpen: (path: string, line?: number) => void;
  onReveal: (path: string) => void;
}) {
  const [open, setOpen] = useState<string | null>(null);
  useEffect(() => setOpen(null), [file.path]);
  const weight = (e: { calls: number; imports: number; refs: number; other: number }) =>
    e.calls + e.imports + e.refs + e.other;
  const uses = data.file_edges
    .filter((e) => e.source === file.path)
    .sort((a, b) => weight(b) - weight(a));
  const usedBy = data.file_edges
    .filter((e) => e.target === file.path)
    .sort((a, b) => weight(b) - weight(a));
  const copyPath = async () => {
    try {
      await navigator.clipboard.writeText(file.path);
      toastInfo("Path copied");
    } catch (e) {
      toastError(e, "clipboard");
    }
  };
  return (
    <>
      <div className="map-side-title mono">{file.path}</div>
      <div className="map-side-actions">
        <button className="map-btn" onClick={() => onOpen(file.path)}>Open in Files</button>
        <button className="map-btn" onClick={() => void copyPath()}>Copy path</button>
      </div>
      {file.symbols.length > 0 && (
        <>
          <div className="map-side-sec">Symbols</div>
          {file.symbols.map((s) => {
            const out = symbolIndex.out.get(s.id) ?? [];
            const into = symbolIndex.into.get(s.id) ?? [];
            const expanded = open === s.id;
            return (
              <div key={s.id} className="map-sym">
                <button
                  className="map-row"
                  onClick={() => setOpen(expanded ? null : s.id)}
                  title={out.length + into.length > 0 ? "Show this symbol's edges" : undefined}
                >
                  <span className="map-sym-kind" aria-hidden>
                    {s.class ? "◇" : s.callable ? "ƒ" : "·"}
                  </span>
                  <span className="map-row-label">{s.label}</span>
                  <span className="map-row-meta">
                    {out.length > 0 && `→${out.length} `}
                    {into.length > 0 && `←${into.length}`}
                  </span>
                  <span
                    className="map-sym-line"
                    role="link"
                    onClick={(e) => {
                      e.stopPropagation();
                      onOpen(file.path, s.line ?? undefined);
                    }}
                  >
                    {s.line !== null ? `L${s.line}` : "open"}
                  </span>
                </button>
                {expanded && (out.length > 0 || into.length > 0) && (
                  <div className="map-sym-edges">
                    {out.map(([id, rel], i) => (
                      <SymbolEdgeRow key={`o${i}`} dir="→" id={id} rel={rel} allSymbols={allSymbols} onOpen={onOpen} onReveal={onReveal} />
                    ))}
                    {into.map(([id, rel], i) => (
                      <SymbolEdgeRow key={`i${i}`} dir="←" id={id} rel={rel} allSymbols={allSymbols} onOpen={onOpen} onReveal={onReveal} />
                    ))}
                  </div>
                )}
              </div>
            );
          })}
        </>
      )}
      {uses.length > 0 && (
        <>
          <div className="map-side-sec">Uses</div>
          {uses.map((e) => (
            <button key={e.target} className="map-row" onClick={() => onReveal(e.target)}>
              <span className="map-row-label">{e.target}</span>
              <span className="map-row-meta">{edgeSummary(e)}</span>
            </button>
          ))}
        </>
      )}
      {usedBy.length > 0 && (
        <>
          <div className="map-side-sec">Used by</div>
          {usedBy.map((e) => (
            <button key={e.source} className="map-row" onClick={() => onReveal(e.source)}>
              <span className="map-row-label">{e.source}</span>
              <span className="map-row-meta">{edgeSummary(e)}</span>
            </button>
          ))}
        </>
      )}
    </>
  );
}

function SymbolEdgeRow({
  dir,
  id,
  rel,
  allSymbols,
  onOpen,
  onReveal,
}: {
  dir: "→" | "←";
  id: string;
  rel: string;
  allSymbols: Map<string, { label: string; line: number | null; file: string }>;
  onOpen: (path: string, line?: number) => void;
  onReveal: (path: string) => void;
}) {
  const target = allSymbols.get(id);
  if (!target) return null;
  return (
    <button
      className="map-row sub"
      title={`${target.file}${target.line !== null ? `:${target.line}` : ""}`}
      onClick={() => {
        onReveal(target.file);
        onOpen(target.file, target.line ?? undefined);
      }}
    >
      <span className="map-sym-kind" aria-hidden>{dir}</span>
      <span className="map-row-label">{target.label}</span>
      <span className="map-row-meta">{rel}</span>
    </button>
  );
}
