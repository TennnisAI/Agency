import { useEffect, useRef, useState } from "react";
import { EditorState } from "@codemirror/state";
import { EditorView, keymap } from "@codemirror/view";
import { basicSetup } from "codemirror";
import { defaultKeymap } from "@codemirror/commands";
import { oneDark } from "@codemirror/theme-one-dark";
import { FileRoot, readFile, writeFile } from "../api";
import { languageExtension } from "../lib/cmLanguage";

export default function FileEditor({ root, path }: { root: FileRoot; path: string }) {
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const [status, setStatus] = useState<"loading" | "binary" | "tooLarge" | "ready" | "error">("loading");
  const [dirty, setDirty] = useState(false);
  const [errorMsg, setErrorMsg] = useState("");

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
          oneDark,
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
  }, [root.kind, root.id, path]);

  return (
    <div className="file-editor-wrap">
      <div className="file-editor-bar">
        <span className="file-editor-path">{path}{dirty ? " ●" : ""}</span>
        <span className="spacer" style={{ flex: 1 }} />
        <button className="git-iconbtn" disabled={!dirty || status !== "ready"} onClick={() => void save.current()}>
          Save
        </button>
      </div>
      {status === "loading" && <div className="diff-empty">loading…</div>}
      {status === "binary" && <div className="diff-empty">Binary file — not shown.</div>}
      {status === "tooLarge" && <div className="diff-empty">File too large to open.</div>}
      {status === "error" && <div className="git-error">{errorMsg}</div>}
      <div ref={hostRef} className="file-editor-host" style={{ display: status === "ready" ? "block" : "none" }} />
    </div>
  );
}
