import { describe, expect, it } from "vitest";
import { MapDir, MapFileEdge } from "../api";
import {
  capLevel,
  countFiles,
  countSymbols,
  edgeSummary,
  findDir,
  findFile,
  layout,
  levelEdges,
  levelNodes,
  parentPath,
  symbolEdgeIndex,
  symbolsById,
} from "./maplayout";

const sym = (id: string, label: string, line: number | null = 1) => ({
  id,
  label,
  line,
  callable: true,
  class: false,
  community: "",
});

const tree: MapDir = {
  name: "",
  path: "",
  dirs: [
    {
      name: "src",
      path: "src",
      dirs: [
        {
          name: "sub",
          path: "src/sub",
          dirs: [],
          files: [{ name: "c.rs", path: "src/sub/c.rs", symbols: [sym("c1", "gamma")] }],
        },
      ],
      files: [
        { name: "a.rs", path: "src/a.rs", symbols: [sym("a1", "alpha"), sym("a2", "beta", 9)] },
        { name: "b.rs", path: "src/b.rs", symbols: [] },
      ],
    },
  ],
  files: [{ name: "top.rs", path: "top.rs", symbols: [sym("t1", "tau")] }],
};

const fileEdge = (source: string, target: string, calls = 0, imports = 0): MapFileEdge => ({
  source,
  target,
  calls,
  imports,
  refs: 0,
  other: 0,
});

describe("tree walking", () => {
  it("counts and finds through nesting", () => {
    expect(countFiles(tree)).toBe(4);
    expect(countSymbols(tree)).toBe(4);
    expect(findDir(tree, "")).toBe(tree);
    expect(findDir(tree, "src/sub")?.files[0].path).toBe("src/sub/c.rs");
    expect(findDir(tree, "src/nope")).toBeNull();
    expect(findFile(tree, "src/a.rs")?.name).toBe("a.rs");
    expect(findFile(tree, "top.rs")?.name).toBe("top.rs");
    expect(findFile(tree, "src/missing.rs")).toBeNull();
    expect(parentPath("src/sub/c.rs")).toBe("src/sub");
    expect(parentPath("top.rs")).toBe("");
  });

  it("lists a level as dirs then files with rollups", () => {
    const nodes = levelNodes(tree);
    expect(nodes.map((n) => [n.key, n.kind, n.files, n.symbols])).toEqual([
      ["src", "dir", 3, 3],
      ["top.rs", "file", 1, 1],
    ]);
    // Boxes have a measurable size before layout runs.
    expect(nodes[0].w).toBeGreaterThan(0);
  });
});

describe("capLevel", () => {
  const wide = (dirs: number, files: number) => [
    ...Array.from({ length: dirs }, (_, i) => ({
      key: `d${i}`, kind: "dir" as const, name: `d${i}`,
      files: 1, symbols: i, x: 0, y: 0, w: 0, h: 0,
    })),
    ...Array.from({ length: files }, (_, i) => ({
      key: `f${i}.ts`, kind: "file" as const, name: `f${i}.ts`,
      files: 1, symbols: i, x: 0, y: 0, w: 0, h: 0,
    })),
  ];

  it("leaves a level that fits completely alone", () => {
    const nodes = wide(2, 3);
    const capped = capLevel(nodes, 10);
    expect(capped.nodes).toBe(nodes);
    expect(capped.hidden).toBe(0);
  });

  it("keeps every directory and the biggest files, and says how many it dropped", () => {
    const { nodes, hidden } = capLevel(wide(3, 10), 6);
    expect(hidden).toBe(7);
    expect(nodes).toHaveLength(6);
    // Directories are how you go deeper, so none of them is ever dropped.
    expect(nodes.filter((n) => n.kind === "dir").map((n) => n.key)).toEqual(["d0", "d1", "d2"]);
    // Then the files with the most symbols.
    expect(nodes.filter((n) => n.kind === "file").map((n) => n.key)).toEqual([
      "f7.ts", "f8.ts", "f9.ts",
    ]);
  });

  it("keeps level order, so trimming never also reshuffles", () => {
    const nodes = wide(2, 8);
    const { nodes: capped } = capLevel(nodes, 5);
    const order = nodes.filter((n) => capped.includes(n));
    expect(capped).toEqual(order);
  });

  it("falls back to the biggest directories when they alone overflow", () => {
    const { nodes, hidden } = capLevel(wide(5, 2), 3);
    expect(hidden).toBe(4);
    expect(nodes.map((n) => n.key)).toEqual(["d2", "d3", "d4"]);
  });
});

describe("levelEdges", () => {
  it("rolls file edges up to the visible boxes", () => {
    const nodes = levelNodes(tree);
    const edges = levelEdges(
      [
        fileEdge("src/a.rs", "top.rs", 2, 1), // dir -> file
        fileEdge("src/sub/c.rs", "top.rs", 1), // nested joins the same box
        fileEdge("src/a.rs", "src/b.rs", 5), // same box: collapses away
        fileEdge("elsewhere.rs", "top.rs", 9), // endpoint with no box: dropped
      ],
      nodes,
    );
    expect(edges).toEqual([
      { source: "src", target: "top.rs", calls: 3, imports: 1, refs: 0, other: 0, weight: 4 },
    ]);
  });
});

describe("layout", () => {
  const nodes = () =>
    ["one", "two", "three", "four", "five", "six"].map((name, i) => ({
      key: name,
      kind: "file" as const,
      name,
      files: 1,
      symbols: i,
      x: 0,
      y: 0,
      w: 90,
      h: 40,
    }));

  it("is deterministic and produces finite, non-overlapping boxes", () => {
    const edges = [
      { source: "one", target: "two", calls: 3, imports: 0, refs: 0, other: 0, weight: 3 },
      { source: "two", target: "three", calls: 1, imports: 0, refs: 0, other: 0, weight: 1 },
    ];
    const a = layout(nodes(), edges);
    const b = layout(nodes(), edges);
    expect(a.map((n) => [n.x, n.y])).toEqual(b.map((n) => [n.x, n.y]));
    for (const n of a) {
      expect(Number.isFinite(n.x)).toBe(true);
      expect(Number.isFinite(n.y)).toBe(true);
    }
    for (let i = 0; i < a.length; i++) {
      for (let j = i + 1; j < a.length; j++) {
        const ox = (a[i].w + a[j].w) / 2 - Math.abs(a[i].x - a[j].x);
        const oy = (a[i].h + a[j].h) / 2 - Math.abs(a[i].y - a[j].y);
        expect(ox <= 0 || oy <= 0).toBe(true);
      }
    }
  });

  it("pulls connected boxes closer than strangers", () => {
    const edges = [
      { source: "one", target: "two", calls: 8, imports: 0, refs: 0, other: 0, weight: 8 },
    ];
    const placed = layout(nodes(), edges);
    const at = new Map(placed.map((n) => [n.key, n]));
    const dist = (p: string, q: string) =>
      Math.hypot(at.get(p)!.x - at.get(q)!.x, at.get(p)!.y - at.get(q)!.y);
    // "six" has no edges at all, so the connected pair should sit closer
    // together than either sits to it.
    expect(dist("one", "two")).toBeLessThan(dist("one", "six"));
    expect(dist("one", "two")).toBeLessThan(dist("two", "six"));
  });

  it("survives empty and single-node levels", () => {
    expect(layout([], [])).toEqual([]);
    const single = layout(nodes().slice(0, 1), []);
    expect(Number.isFinite(single[0].x)).toBe(true);
  });
});

describe("edge summaries", () => {
  it("summarises counts in words", () => {
    expect(edgeSummary({ calls: 2, imports: 1, refs: 0, other: 0 })).toBe("2 calls, 1 import");
    expect(edgeSummary({ calls: 1, imports: 0, refs: 3, other: 2 })).toBe("1 call, 3 refs, 2 other");
    expect(edgeSummary({ calls: 0, imports: 0, refs: 0, other: 0 })).toBe("");
  });
});

describe("symbol indexes", () => {
  it("indexes edges both ways and symbols by id", () => {
    const idx = symbolEdgeIndex([
      ["a1", "t1", "calls"],
      ["a1", "c1", "references"],
      ["t1", "a1", "calls"],
    ]);
    expect(idx.out.get("a1")).toEqual([
      ["t1", "calls"],
      ["c1", "references"],
    ]);
    expect(idx.into.get("a1")).toEqual([["t1", "calls"]]);
    const syms = symbolsById(tree);
    expect(syms.get("c1")).toEqual({ label: "gamma", line: 1, file: "src/sub/c.rs" });
    expect(syms.size).toBe(4);
  });
});
