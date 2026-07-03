import { useEffect, useRef, useState } from "react";
import { Compartment, EditorState } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { basicSetup } from "codemirror";
import { defaultKeymap } from "@codemirror/commands";
import { marked } from "marked";
import { FileRoot, readFile, readFileBase64, writeFile } from "../api";
import { languageExtension } from "../lib/cmLanguage";
import { editorChromeTheme, editorHighlight } from "../lib/cmTheme";
import { getWordWrap } from "../lib/editorPrefs";

// How a file is displayed. Raster images and PDFs render directly (no code
// view); md/html/svg open in the editor with a Preview toggle; everything
// else is plain text (or "binary — not shown").
type ViewKind =
  | { kind: "image" }
  | { kind: "pdf" }
  | { kind: "text"; preview: "md" | "html" | "svg" | null };

function viewKind(path: string): ViewKind {
  const ext = (path.split(".").pop() ?? "").toLowerCase();
  if (["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "avif"].includes(ext)) return { kind: "image" };
  if (ext === "pdf") return { kind: "pdf" };
  if (ext === "md" || ext === "markdown") return { kind: "text", preview: "md" };
  if (ext === "html" || ext === "htm") return { kind: "text", preview: "html" };
  if (ext === "svg") return { kind: "text", preview: "svg" };
  return { kind: "text", preview: null };
}

// Markdown preview document, themed from the live CSS variables so it follows
// the active app theme.
function markdownSrcDoc(html: string): string {
  const v = (name: string) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return `<!doctype html><html><head><meta charset="utf-8"><style>
:root { color-scheme: dark; }
body { font-family: -apple-system, system-ui, sans-serif; font-size: 14px; line-height: 1.65;
  background: ${v("--base")}; color: ${v("--text")}; margin: 0 auto; max-width: 760px; padding: 26px 30px 60px; }
h1, h2 { border-bottom: 1px solid ${v("--line")}; padding-bottom: 6px; }
a { color: ${v("--blue")}; }
code, pre { font-family: ui-monospace, Menlo, monospace; font-size: 12.5px; background: ${v("--crust")}; border-radius: 6px; }
code { padding: 1px 5px; }
pre { padding: 12px 14px; overflow: auto; } pre code { padding: 0; }
blockquote { border-left: 3px solid ${v("--s1")}; margin-left: 0; padding-left: 14px; color: ${v("--sub0")}; }
img { max-width: 100%; }
table { border-collapse: collapse; } th, td { border: 1px solid ${v("--line")}; padding: 6px 10px; }
hr { border: none; border-top: 1px solid ${v("--line")}; }
</style></head><body>${html}</body></html>`;
}

export default function FileEditor({ root, path }: { root: FileRoot; path: string }) {
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  // Compartment so the word-wrap toggle reconfigures the live editor in place.
  const wrapRef = useRef(new Compartment());
  const [status, setStatus] = useState<"loading" | "binary" | "tooLarge" | "ready" | "error">("loading");
  const [dirty, setDirty] = useState(false);
  const [errorMsg, setErrorMsg] = useState("");
  // Binary payload (data URL) for image/pdf files.
  const [dataUrl, setDataUrl] = useState("");
  // Preview toggle for md/html/svg; holds the rendered content when active.
  const [previewing, setPreviewing] = useState(false);
  const [previewContent, setPreviewContent] = useState("");

  const vk = viewKind(path);

  // Keep a stable save handler that reads the current doc from the live view.
  const save = useRef(async () => {});
  save.current = async () => {
    const view = viewRef.current;
    if (!view) return;
    try {
      await writeFile(root, path, view.state.doc.toString());
      setDirty(false);
    } catch (e) {
      setErrorMsg(String(e));
      setStatus("error");
    }
  };

  useEffect(() => {
    let cancelled = false;
    setStatus("loading");
    setDirty(false);
    setErrorMsg("");
    setDataUrl("");
    setPreviewing(false);
    setPreviewContent("");

    // Images and PDFs skip the text pipeline entirely.
    if (vk.kind === "image" || vk.kind === "pdf") {
      readFileBase64(root, path).then((bc) => {
        if (cancelled) return;
        if (bc.tooLarge) { setStatus("tooLarge"); return; }
        setDataUrl(`data:${bc.mime};base64,${bc.b64}`);
        setStatus("ready");
      }).catch((e) => {
        if (cancelled) return;
        setErrorMsg(String(e));
        setStatus("error");
      });
      return () => { cancelled = true; };
    }

    readFile(root, path).then((fc) => {
      if (cancelled) return;
      if (fc.tooLarge) { setStatus("tooLarge"); return; }
      if (fc.binary) { setStatus("binary"); return; }
      setStatus("ready");
      const host = hostRef.current;
      if (!host) { setStatus("error"); setErrorMsg("Editor failed to mount."); return; }
      viewRef.current?.destroy();
      const state = EditorState.create({
        doc: fc.text,
        extensions: [
          basicSetup,
          editorChromeTheme,
          editorHighlight,
          wrapRef.current.of(getWordWrap() ? EditorView.lineWrapping : []),
          ...languageExtension(path),
          keymap.of([
            { key: "Mod-s", preventDefault: true, run: () => { void save.current(); return true; } },
            ...defaultKeymap,
          ]),
          EditorView.updateListener.of((u) => { if (u.docChanged) setDirty(true); }),
        ],
      });
      viewRef.current = new EditorView({ state, parent: host });
    }).catch((e) => {
      if (cancelled) return;
      setErrorMsg(String(e));
      setStatus("error");
    });

    return () => {
      cancelled = true;
      viewRef.current?.destroy();
      viewRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [root.kind, root.id, path]);

  // Apply word-wrap toggles to the open editor without reloading the file.
  useEffect(() => {
    const onWrap = (e: Event) => {
      const on = (e as CustomEvent<boolean>).detail;
      viewRef.current?.dispatch({
        effects: wrapRef.current.reconfigure(on ? EditorView.lineWrapping : []),
      });
    };
    window.addEventListener("wordwrapchange", onWrap);
    return () => window.removeEventListener("wordwrapchange", onWrap);
  }, []);

  // Render the preview from the live editor doc (unsaved edits included).
  function showPreview() {
    const doc = viewRef.current?.state.doc.toString() ?? "";
    if (vk.kind === "text" && vk.preview === "md") {
      setPreviewContent(markdownSrcDoc(marked.parse(doc, { async: false }) as string));
    } else if (vk.kind === "text" && vk.preview === "html") {
      setPreviewContent(doc);
    } else if (vk.kind === "text" && vk.preview === "svg") {
      setPreviewContent(`data:image/svg+xml;charset=utf-8,${encodeURIComponent(doc)}`);
    }
    setPreviewing(true);
  }

  const previewable = vk.kind === "text" && vk.preview !== null;

  return (
    <div className="file-editor-wrap">
      <div className="file-editor-bar">
        <span className="file-editor-path">{path}{dirty ? " ●" : ""}</span>
        <span className="spacer" style={{ flex: 1 }} />
        {previewable && status === "ready" && (
          <div className="seg seg-mini">
            <button className={previewing ? "" : "on"} onClick={() => setPreviewing(false)}>Code</button>
            <button className={previewing ? "on" : ""} onClick={showPreview}>Preview</button>
          </div>
        )}
        {vk.kind === "text" && (
          <button className="git-iconbtn" disabled={!dirty || status !== "ready"} onClick={() => void save.current()}>
            Save
          </button>
        )}
      </div>
      {status === "loading" && <div className="diff-empty">loading…</div>}
      {status === "binary" && <div className="diff-empty">Binary file — no preview for this format.</div>}
      {status === "tooLarge" && <div className="diff-empty">File too large to open.</div>}
      {status === "error" && <div className="git-error">{errorMsg}</div>}

      {vk.kind === "image" && status === "ready" && (
        <div className="file-preview-media"><img src={dataUrl} alt={path} /></div>
      )}
      {vk.kind === "pdf" && status === "ready" && (
        <iframe className="file-preview-frame" title={path} src={dataUrl} />
      )}

      {vk.kind === "text" && (
        <>
          <div
            ref={hostRef}
            className="file-editor-host"
            style={{ display: status === "ready" && !previewing ? "block" : "none" }}
          />
          {previewing && status === "ready" && (
            vk.preview === "svg" ? (
              <div className="file-preview-media"><img src={previewContent} alt={path} /></div>
            ) : (
              // sandbox="" (no allow-* tokens) blocks scripts and same-origin
              // access — repo files are agent-written, so previews must never
              // execute in the app's IPC-capable context.
              <iframe className="file-preview-frame" title={path} sandbox="" srcDoc={previewContent} />
            )
          )}
        </>
      )}
    </div>
  );
}
