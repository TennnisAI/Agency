//! The Map's view model, computed from graphify's on-disk `graph.json`
//! (networkx node-link JSON: `nodes` + `links`, flat, symbol-level). The raw
//! file is the wrong shape and the wrong size for the UI: 6.9 MB and 13k links
//! for this repository alone, most of it per-edge metadata the viewer never
//! reads. This module reduces it to the three things the Map actually renders:
//! a directory → file → symbol tree, file-level dependency edges with category
//! counts, and the symbol-level edges behind the detail panel.
//!
//! Pure text-in, value-out so the whole reduction is unit-testable without a
//! repo or a built graph; reading the file and finding the repo is the
//! caller's job (state.rs).
//!
//! Two observed quirks of graphify's output shape this code:
//! - `source_file` is relative to wherever the build ran. Run as `graphify .`
//!   in the checkout (Agency's own default) it is repo-relative, but a build
//!   pointed at a directory by name prefixes every path with that name
//!   ("agency-copy/ui/src/api.ts" observed from `graphify update agency-copy`),
//!   so the caller passes the repo directory name and it is stripped here.
//!   Only when *every* path carries it, though; see `view`.
//! - Files appear as nodes themselves (label "src/git.rs", non-callable, L1),
//!   carrying the file-to-file `imports` edges. They are kept for edge
//!   aggregation but excluded from a file's symbol list, where a row named
//!   like the file it sits in reads as a bug.

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Everything the Map needs for one project, in one payload.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct GraphView {
    pub root: Dir,
    /// Cross-file dependencies, aggregated per (source, target) file pair.
    pub file_edges: Vec<FileEdge>,
    /// (source symbol id, target symbol id, relation) for the detail panel.
    /// Symbol-to-symbol only; file-node endpoints are already in `file_edges`.
    pub symbol_edges: Vec<(String, String, String)>,
    pub stats: Stats,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Dir {
    pub name: String,
    /// Repo-relative, "" for the root.
    pub path: String,
    pub dirs: Vec<Dir>,
    pub files: Vec<File>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct File {
    pub name: String,
    pub path: String,
    pub symbols: Vec<Symbol>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Symbol {
    pub id: String,
    pub label: String,
    /// 1-based line of the definition, when graphify recorded one ("L794").
    pub line: Option<u32>,
    pub callable: bool,
    pub class: bool,
    pub community: String,
}

/// One direction of one file pair; (B, A) is a separate edge from (A, B).
/// The counts are collapsed into the categories the viewer distinguishes.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FileEdge {
    pub source: String,
    pub target: String,
    pub calls: u32,
    pub imports: u32,
    pub refs: u32,
    pub other: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Stats {
    pub files: u32,
    pub symbols: u32,
    pub edges: u32,
    pub communities: u32,
}

/// Reduce a `graph.json` to the Map view model. `repo_dir` is the checkout
/// directory's name, stripped off `source_file` paths when a build was pointed
/// at the directory by name rather than run inside it, which is recognised by
/// the prefix being on every path rather than merely on some.
pub fn view(text: &str, repo_dir: Option<&str>) -> anyhow::Result<GraphView> {
    let doc: serde_json::Value = serde_json::from_str(text)?;
    let nodes = doc
        .get("nodes")
        .and_then(|n| n.as_array())
        .ok_or_else(|| anyhow::anyhow!("graph.json has no nodes array"))?;
    // NetworkX <= 3.1 serialises edges as "links"; newer exports say "edges".
    let links = doc
        .get("links")
        .or_else(|| doc.get("edges"))
        .and_then(|l| l.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[]);

    // First pass: the code nodes carrying a usable path. Collected before
    // anything is built, because the build-directory prefix below can only be
    // told apart from a real directory by looking at all of them at once.
    let mut coded: Vec<(&str, &serde_json::Value, String)> = Vec::new();
    for node in nodes {
        let Some(id) = node.get("id").and_then(|v| v.as_str()) else { continue };
        // External/package entries carry a `type`; docs, concepts and
        // rationale notes carry another `file_type`. The Map is a code map.
        if node.get("type").is_some_and(|t| !t.is_null()) {
            continue;
        }
        if node.get("file_type").and_then(|v| v.as_str()) != Some("code") {
            continue;
        }
        let Some(path) = node.get("source_file").and_then(|v| v.as_str()).and_then(clean_path)
        else {
            continue;
        };
        coded.push((id, node, path));
    }

    // A build pointed at a directory by name prefixes every path with that
    // name; a build run inside the checkout writes repo-relative paths.
    // Stripping on a bare prefix match confused the two: a checkout that
    // merely happens to be *named* like one of its own top-level directories
    // (a clone in ~/code/src, say) had that directory cut off every path under
    // it, so "src/a.rs" became "a.rs", files merged with same-named ones at
    // the root, and "Open in Files" asked for a path that does not exist.
    // Requiring the prefix on *every* path tells the two apart: the build
    // directory's name is on all of them or it is not the build directory.
    let strip = repo_dir
        .filter(|d| !d.is_empty())
        .map(|d| format!("{d}/"))
        .filter(|p| !coded.is_empty() && coded.iter().all(|(_, _, path)| path.starts_with(p)));

    // Second pass: every kept node, keyed by id. `symbol: None` marks the
    // nodes that stand for a file itself.
    struct Kept {
        file: String,
        symbol: Option<Symbol>,
    }
    let mut kept: HashMap<String, Kept> = HashMap::new();
    for (id, node, path) in coded {
        let file = match strip.as_deref() {
            Some(prefix) => path.strip_prefix(prefix).unwrap_or(&path).to_string(),
            None => path,
        };
        let label = node.get("label").and_then(|v| v.as_str()).unwrap_or(id).to_string();
        let callable = node.get("_callable").and_then(|v| v.as_bool()).unwrap_or(false);
        let symbol = if !callable && names_the_file(&file, &label) {
            None
        } else {
            Some(Symbol {
                id: id.to_string(),
                label,
                line: node.get("source_location").and_then(|v| v.as_str()).and_then(parse_line),
                callable,
                class: node.get("_callable_class").and_then(|v| v.as_bool()).unwrap_or(false),
                community: node
                    .get("community_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            })
        };
        kept.insert(id.to_string(), Kept { file, symbol });
    }

    // Third pass: links between kept nodes. `contains` is nesting (file to
    // symbol, class to inner fn), which the tree already encodes.
    let mut file_edges: BTreeMap<(String, String), FileEdge> = BTreeMap::new();
    let mut symbol_edges: Vec<(String, String, String)> = Vec::new();
    let mut edge_count: u32 = 0;
    for link in links {
        let (Some(source), Some(target), Some(relation)) = (
            link.get("source").and_then(|v| v.as_str()),
            link.get("target").and_then(|v| v.as_str()),
            link.get("relation").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        if relation == "contains" {
            continue;
        }
        let (Some(s), Some(t)) = (kept.get(source), kept.get(target)) else { continue };
        edge_count += 1;
        if s.symbol.is_some() && t.symbol.is_some() {
            symbol_edges.push((source.to_string(), target.to_string(), relation.to_string()));
        }
        if s.file != t.file {
            let edge =
                file_edges.entry((s.file.clone(), t.file.clone())).or_insert_with(|| FileEdge {
                    source: s.file.clone(),
                    target: t.file.clone(),
                    calls: 0,
                    imports: 0,
                    refs: 0,
                    other: 0,
                });
            match relation {
                "calls" | "indirect_call" => edge.calls += 1,
                "imports" | "imports_from" | "dynamic_import" => edge.imports += 1,
                "references" => edge.refs += 1,
                _ => edge.other += 1,
            }
        }
    }
    symbol_edges.sort();

    // Fourth pass: the tree. BTreeMap keeps every listing deterministic.
    let mut files: BTreeMap<String, Vec<Symbol>> = BTreeMap::new();
    let mut communities: BTreeSet<String> = BTreeSet::new();
    for k in kept.into_values() {
        let symbols = files.entry(k.file).or_default();
        if let Some(s) = k.symbol {
            if !s.community.is_empty() {
                communities.insert(s.community.clone());
            }
            symbols.push(s);
        }
    }
    let stats = Stats {
        files: files.len() as u32,
        symbols: files.values().map(|s| s.len() as u32).sum(),
        edges: edge_count,
        communities: communities.len() as u32,
    };
    Ok(GraphView {
        root: build_tree(files),
        file_edges: file_edges.into_values().collect(),
        symbol_edges,
        stats,
    })
}

/// Normalise a `source_file` to a checkout-relative path, or None for one that
/// cannot be inside the checkout (absolute, drive-lettered, or escaping via
/// ".."). The build-directory prefix is *not* stripped here: that decision
/// needs every path at once and is made in `view`.
fn clean_path(raw: &str) -> Option<String> {
    let p = raw.replace('\\', "/");
    let p = p.strip_prefix("./").unwrap_or(&p);
    if p.is_empty() || p.starts_with('/') || p.contains(':') {
        return None;
    }
    // npm-scoped import targets ("@codemirror/state") arrive as code-typed
    // file nodes with the package as their path, and drew a phantom
    // "@codemirror/" directory on the map.
    if p.starts_with('@') {
        return None;
    }
    if p.split('/').any(|c| c == ".." || c.is_empty()) {
        return None;
    }
    Some(p.to_string())
}

/// Whether `label` names the file itself rather than a symbol inside it:
/// graphify emits a non-callable node per file, labelled with the file's own
/// path ("src/git.rs"). Matched on whole path components, because a bare
/// `file.ends_with(label)` swallowed real symbols -- a non-callable "ts" in
/// "src/main.ts" disappeared from that file's symbol list.
fn names_the_file(file: &str, label: &str) -> bool {
    file == label || file.strip_suffix(label).is_some_and(|head| head.ends_with('/'))
}

/// "L794" → 794. Anything else graphify writes here is left as no line.
fn parse_line(loc: &str) -> Option<u32> {
    let digits: String =
        loc.strip_prefix('L')?.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

fn build_tree(files: BTreeMap<String, Vec<Symbol>>) -> Dir {
    let mut root =
        Dir { name: String::new(), path: String::new(), dirs: Vec::new(), files: Vec::new() };
    for (path, mut symbols) in files {
        symbols.sort_by(|a, b| (a.line, &a.label).cmp(&(b.line, &b.label)));
        let mut dir = &mut root;
        let mut walked = String::new();
        let mut parts = path.split('/').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                dir.files.push(File { name: part.to_string(), path: path.clone(), symbols });
                break;
            }
            if !walked.is_empty() {
                walked.push('/');
            }
            walked.push_str(part);
            // BTreeMap iteration is sorted, so a needed child is either the
            // last one pushed or new; this stays O(paths × depth).
            let missing = dir.dirs.last().map(|d| d.path != walked).unwrap_or(true);
            if missing {
                dir.dirs.push(Dir {
                    name: part.to_string(),
                    path: walked.clone(),
                    dirs: Vec::new(),
                    files: Vec::new(),
                });
            }
            dir = dir.dirs.last_mut().unwrap();
        }
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: &str, label: &str, file: &str, loc: &str, callable: bool) -> serde_json::Value {
        json!({
            "id": id, "label": label, "_callable": callable, "_origin": "ast",
            "community": 1, "community_name": "core", "file_type": "code",
            "norm_label": label.to_lowercase(), "source_file": file, "source_location": loc,
        })
    }

    fn link(source: &str, target: &str, relation: &str) -> serde_json::Value {
        json!({ "source": source, "target": target, "relation": relation })
    }

    fn collect(d: &Dir, out: &mut Vec<String>) {
        out.extend(d.files.iter().map(|f| f.path.clone()));
        d.dirs.iter().for_each(|s| collect(s, out));
    }

    fn graph(nodes: Vec<serde_json::Value>, links: Vec<serde_json::Value>) -> String {
        json!({ "directed": false, "multigraph": false, "graph": {}, "nodes": nodes, "links": links })
            .to_string()
    }

    #[test]
    fn builds_tree_with_sorted_symbols() {
        let text = graph(
            vec![
                node("b", "beta", "src/a.rs", "L20", true),
                node("a", "Alpha", "src/a.rs", "L5", false),
                node("c", "gamma", "src/sub/c.rs", "L1", true),
                node("d", "delta", "top.rs", "L2", true),
            ],
            vec![],
        );
        let v = view(&text, None).unwrap();
        assert_eq!(v.root.files.len(), 1); // top.rs
        assert_eq!(v.root.files[0].path, "top.rs");
        let src = &v.root.dirs[0];
        assert_eq!((src.name.as_str(), src.path.as_str()), ("src", "src"));
        let a = &src.files[0];
        assert_eq!(a.path, "src/a.rs");
        // Sorted by line, not input order.
        assert_eq!(
            a.symbols.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(),
            ["Alpha", "beta"]
        );
        assert_eq!(a.symbols[0].line, Some(5));
        assert!(!a.symbols[0].callable);
        assert_eq!(src.dirs[0].files[0].path, "src/sub/c.rs");
        assert_eq!(v.stats, Stats { files: 3, symbols: 4, edges: 0, communities: 1 });
    }

    #[test]
    fn strips_build_dir_prefixes() {
        // Observed from `graphify update agency-copy`: every path carries the
        // directory name the build was pointed at. `./` is the run-inside form
        // and appears on those paths too.
        let text = graph(
            vec![
                node("a", "alpha", "agency-copy/src/a.rs", "L1", true),
                node("b", "beta", "./agency-copy/src/b.rs", "L1", true),
            ],
            vec![],
        );
        let v = view(&text, Some("agency-copy")).unwrap();
        let src = &v.root.dirs[0];
        let paths: Vec<&str> = src.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["src/a.rs", "src/b.rs"]);
        assert_eq!(v.stats.files, 2);
    }

    #[test]
    fn drops_paths_that_cannot_be_inside_the_checkout() {
        let text = graph(
            vec![
                node("ok", "alpha", "src/a.rs", "L1", true),
                node("abs", "abs", "/etc/passwd", "L1", true),
                node("esc", "esc", "../outside.rs", "L1", true),
                node("win", "win", "C:\\repo\\x.rs", "L1", true),
                node("npm", "state", "@codemirror/state", "L1", true),
                node("dots", "dots", "src/../../etc/shadow", "L1", true),
            ],
            vec![],
        );
        let v = view(&text, None).unwrap();
        let mut paths = Vec::new();
        collect(&v.root, &mut paths);
        assert_eq!(paths, ["src/a.rs"], "absolute, escaping, drive and package paths are dropped");
    }

    #[test]
    fn keeps_a_repo_dir_name_that_is_a_real_directory() {
        // The checkout lives in a directory called "src" and the build ran
        // inside it, so the paths are already repo-relative. Stripping on a
        // bare prefix match cut "src/" off every path under it.
        let text = graph(
            vec![
                node("a", "alpha", "src/a.rs", "L1", true),
                node("b", "beta", "lib/b.rs", "L1", true),
            ],
            vec![],
        );
        let v = view(&text, Some("src")).unwrap();
        let mut paths = Vec::new();
        collect(&v.root, &mut paths);
        paths.sort();
        assert_eq!(paths, ["lib/b.rs", "src/a.rs"], "the prefix is not on every path");
        // The same name really is the build directory when it is on all of
        // them, and then it is still stripped.
        let text = graph(
            vec![
                node("a", "alpha", "src/a.rs", "L1", true),
                node("b", "beta", "src/lib/b.rs", "L1", true),
            ],
            vec![],
        );
        let v = view(&text, Some("src")).unwrap();
        let mut paths = Vec::new();
        collect(&v.root, &mut paths);
        paths.sort();
        assert_eq!(paths, ["a.rs", "lib/b.rs"]);
    }

    #[test]
    fn a_symbol_named_like_a_path_suffix_is_still_a_symbol() {
        // `file.ends_with(label)` swallowed these: "src/main.ts" ends with
        // "ts", so a non-callable symbol of that name read as the file node.
        let text = graph(
            vec![
                node("s", "ts", "src/main.ts", "L3", false),
                node("f", "src/main.ts", "src/main.ts", "L1", false),
                node("b", "main.ts", "src/main.ts", "L1", false),
            ],
            vec![],
        );
        let v = view(&text, None).unwrap();
        let labels: Vec<&str> =
            v.root.dirs[0].files[0].symbols.iter().map(|s| s.label.as_str()).collect();
        // The whole path and the bare basename both name the file; "ts" does not.
        assert_eq!(labels, ["ts"]);
    }

    #[test]
    fn filters_non_code_nodes_and_their_links() {
        let mut doc_node = node("doc", "README.md", "README.md", "L1", false);
        doc_node["file_type"] = json!("document");
        let mut pkg = node("pkg", "@scope/pkg", "@scope/pkg", "L1", false);
        pkg["type"] = json!("package");
        let text = graph(
            vec![node("a", "alpha", "src/a.rs", "L1", true), doc_node, pkg],
            vec![link("a", "doc", "references"), link("a", "pkg", "imports")],
        );
        let v = view(&text, None).unwrap();
        assert_eq!(v.stats.files, 1);
        assert!(v.file_edges.is_empty());
        assert!(v.symbol_edges.is_empty());
        assert_eq!(v.stats.edges, 0);
    }

    #[test]
    fn file_nodes_feed_edges_but_not_symbol_lists() {
        // graphify emits the file itself as a node (label "src/git.rs",
        // non-callable, L1) and hangs the file-to-file imports off it.
        let text = graph(
            vec![
                node("fa", "src/a.rs", "src/a.rs", "L1", false),
                node("fb", "src/b.rs", "src/b.rs", "L1", false),
                node("s", "worker", "src/a.rs", "L10", true),
            ],
            vec![link("fa", "fb", "imports"), link("fa", "s", "contains")],
        );
        let v = view(&text, None).unwrap();
        let src = &v.root.dirs[0];
        assert_eq!(
            src.files[0].symbols.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(),
            ["worker"]
        );
        assert!(src.files[1].symbols.is_empty());
        assert_eq!(v.file_edges.len(), 1);
        assert_eq!((v.file_edges[0].source.as_str(), v.file_edges[0].imports), ("src/a.rs", 1));
        // File-node endpoints stay out of the symbol-level list, and contains
        // is dropped entirely.
        assert!(v.symbol_edges.is_empty());
        assert_eq!(v.stats.edges, 1);
    }

    #[test]
    fn aggregates_file_edges_by_category_and_direction() {
        let text = graph(
            vec![
                node("a1", "one", "a.rs", "L1", true),
                node("a2", "two", "a.rs", "L9", true),
                node("b1", "three", "b.rs", "L1", true),
            ],
            vec![
                link("a1", "b1", "calls"),
                link("a2", "b1", "indirect_call"),
                link("a1", "b1", "imports_from"),
                link("a1", "b1", "references"),
                link("a1", "b1", "extends"),
                link("b1", "a1", "calls"),
                link("a1", "a2", "calls"), // intra-file: symbol edge only
            ],
        );
        let v = view(&text, None).unwrap();
        assert_eq!(v.file_edges.len(), 2);
        let ab = &v.file_edges[0];
        assert_eq!(
            (ab.source.as_str(), ab.target.as_str(), ab.calls, ab.imports, ab.refs, ab.other),
            ("a.rs", "b.rs", 2, 1, 1, 1)
        );
        let ba = &v.file_edges[1];
        assert_eq!((ba.source.as_str(), ba.target.as_str(), ba.calls), ("b.rs", "a.rs", 1));
        assert_eq!(v.symbol_edges.len(), 7);
        assert!(v.symbol_edges.contains(&("a1".into(), "a2".into(), "calls".into())));
        assert_eq!(v.stats.edges, 7);
    }

    #[test]
    fn accepts_edges_key_and_missing_location() {
        let mut n = node("a", "alpha", "a.rs", "", true);
        n.as_object_mut().unwrap().remove("source_location");
        let text = json!({ "nodes": [n], "edges": [] }).to_string();
        let v = view(&text, None).unwrap();
        assert_eq!(v.root.files[0].symbols[0].line, None);
    }

    #[test]
    fn parse_line_reads_graphify_forms() {
        assert_eq!(parse_line("L794"), Some(794));
        assert_eq!(parse_line("L10-L20"), Some(10));
        assert_eq!(parse_line("794"), None);
        assert_eq!(parse_line("Lx"), None);
    }

    #[test]
    fn rejects_malformed_documents() {
        assert!(view("not json", None).is_err());
        assert!(view("{\"links\": []}", None).is_err(), "no nodes array");
    }
}
