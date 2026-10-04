import { MutableRefObject, ReactNode, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { RunStoreProvider, useRuns } from "./store/runs";
import AgentFocus from "./components/AgentFocus";
import FileEditor, { FileEditorHandle } from "./components/FileEditor";
import DocsEditor, { DocsEditorHandle } from "./components/DocsEditor";
import WindowControls from "./components/WindowControls";
import Toasts from "./components/Toasts";
import { ReattachGlyph } from "./components/PoppedOut";
import { listRuns, navigateMain, popoutSelf, reattachPopout, setUiState } from "./api";
import { runListLabel } from "./agents";
import { PopoutTarget, popoutTitle } from "./lib/popout";
import { WINDOW_LABEL } from "./lib/windowRole";
import { IS_LINUX, IS_TAURI } from "./lib/platform";
import { NAVIGATE_EVENT, NavTarget, requestNavigate } from "./lib/navigate";
import { requestFind, requestFindStep } from "./lib/findBus";
import { bufferKey, stashBuffer } from "./lib/editorBuffers";
import { applyTheme, STORAGE_KEY as THEME_KEY } from "./lib/themes";
import { useDocs } from "./hooks/useDocs";
import { useCrossRefs } from "./hooks/useCrossRefs";
import { resolveTarget } from "./lib/links";
import { toastInfo } from "./lib/toast";
import { unlistenQuietly } from "./lib/unlisten";

// What the window hands back when its item goes home: a file's unsaved edits,
// or null. Set by whichever body is mounted; a note saves itself first instead.
type Collect = MutableRefObject<() => Promise<string | null>>;

/**
 * The root of a popped-out window (AGE-252): one agent, file or note, a title
 * bar with the way back to the main window, and nothing else. Everything this
 * window cannot show itself (a link to another note, an issue chip, Approve)
 * is handed to the main window, which raises itself to show it.
 */
export default function PopoutApp() {
  // undefined while asking; null when this window has nothing registered (its
  // item already went back, and the window outlived it).
  const [hello, setHello] = useState<{ target: PopoutTarget; draft: string | null } | null | undefined>(undefined);
  const [title, setTitle] = useState("");
  const [leaving, setLeaving] = useState(false);
  const collect: Collect = useRef(async () => null);
  const leavingRef = useRef(false);

  useEffect(() => {
    popoutSelf()
      .then((h) => {
        setHello(h);
        if (h) setTitle(popoutTitle(h.target));
      })
      .catch(() => setHello(null));
  }, []);

  // Hand the item back. `show` brings it up in the main window; a plain close
  // of this window sends it home without taking the main window anywhere.
  // Unmounting the body first is what detaches a terminal from here; the
  // backend refuses a detach that lands after the main window has re-attached,
  // so the order cannot cost the main window its stream.
  const goBack = async (show: boolean) => {
    if (leavingRef.current) return;
    leavingRef.current = true;
    const draft = await collect.current().catch(() => null);
    setLeaving(true);
    try {
      await reattachPopout(WINDOW_LABEL, draft, show);
    } catch {
      // Nothing to hand back to: close regardless, or the window is stuck.
      await getCurrentWindow().destroy().catch(() => {});
    }
  };
  const goBackRef = useRef(goBack);
  goBackRef.current = goBack;

  useEffect(() => {
    if (!IS_TAURI) return;
    const win = getCurrentWindow();
    const subs = [
      // The window's own close button (and Window ▸ Close): the item goes home.
      win.onCloseRequested((e) => {
        e.preventDefault();
        void goBackRef.current(false);
      }),
      // The main window's "Bring back here".
      listen("popout-reattach-request", () => { void goBackRef.current(true); }),
      // Find is the only menu action routed to a popout (menu.rs).
      listen<string>("menu", (e) => {
        switch (e.payload) {
          case "find": requestFind("find"); break;
          case "replace": requestFind("replace"); break;
          case "find-next": requestFindStep(false); break;
          case "find-prev": requestFindStep(true); break;
        }
      }),
    ];
    // Links out of this window's item land in the main window.
    const navigate = (e: Event) => {
      const detail = (e as CustomEvent<NavTarget>).detail;
      if (detail) navigateMain(detail).catch(() => {});
    };
    window.addEventListener(NAVIGATE_EVENT, navigate);
    // A theme picked in the main window's Settings. Both windows share one
    // localStorage, and a write in one is a `storage` event in the other.
    const onStorage = (e: StorageEvent) => {
      if (e.key === THEME_KEY && e.newValue) applyTheme(e.newValue);
    };
    window.addEventListener("storage", onStorage);
    // ⌘F when the menu bar does not get it first (see App.tsx).
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.shiftKey || e.key.toLowerCase() !== "f") return;
      if (!requestFind(e.altKey ? "replace" : "find")) return;
      e.preventDefault();
      e.stopPropagation();
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      subs.forEach((s) => s.then(unlistenQuietly).catch(() => {}));
      window.removeEventListener(NAVIGATE_EVENT, navigate);
      window.removeEventListener("storage", onStorage);
      window.removeEventListener("keydown", onKey, true);
    };
  }, []);

  // The OS window title: what the Window menu, the dock and the app switcher
  // call this window.
  useEffect(() => {
    if (IS_TAURI && title) getCurrentWindow().setTitle(title).catch(() => {});
    document.title = title || "Agency";
  }, [title]);

  const target = hello?.target;
  let body: ReactNode;
  if (hello === undefined) {
    body = <div className="app-loading"><span className="spinner" /></div>;
  } else if (!target || leaving) {
    body = <div className="board empty">{leaving ? "Moving back to the main window…" : "Nothing to show here. It went back to the main window."}</div>;
  } else if (target.kind === "run") {
    body = (
      <RunStoreProvider>
        <PopoutRun target={target} onTitle={setTitle} onGone={() => void goBack(false)} />
      </RunStoreProvider>
    );
  } else if (target.kind === "file") {
    body = <PopoutFile target={target} draft={hello.draft} collect={collect} />;
  } else {
    body = <PopoutNote target={target} collect={collect} onTitle={setTitle} />;
  }

  return (
    <div className="shell popout">
      {/* The same drag handle as the main title bar (see TitleBar). */}
      <header className="titlebar popout-bar" data-tauri-drag-region="deep">
        <div className="titlebar-left">
          <span className="popout-title" title={title}>{title}</span>
        </div>
        <div className="titlebar-right">
          {target && !leaving && (
            <button
              className="popout-back"
              title="Put this back in the main window"
              onClick={() => void goBack(true)}
            >
              <ReattachGlyph />
              <span>Bring back</span>
            </button>
          )}
          {IS_LINUX && IS_TAURI && <WindowControls />}
        </div>
      </header>
      <div className="popout-body">{body}</div>
      <Toasts />
    </div>
  );
}

function PopoutRun({
  target,
  onTitle,
  onGone,
}: {
  target: Extract<PopoutTarget, { kind: "run" }>;
  onTitle: (t: string) => void;
  onGone: () => void;
}) {
  const {
    runs, setSelectedProject, setFocusedRun, setView, approveRunId, setApproveRun, onScreenRunId,
  } = useRuns();
  // Did the run exist when this window opened? Null while asking.
  const [found, setFound] = useState<boolean | null>(null);
  const seen = useRef(false);

  useEffect(() => {
    // setSelectedProject clears the focus, so the focus goes after it; React
    // batches the three and the later writes win.
    setSelectedProject(target.projectId);
    setFocusedRun(target.runId);
    setView("focus");
    listRuns(target.projectId)
      .then((rs) => setFound(rs.some((r) => r.id === target.runId)))
      .catch(() => setFound(false));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target.projectId, target.runId]);

  const run = runs.find((r) => r.id === target.runId) ?? null;
  const label = run ? runListLabel(run) : "";
  useEffect(() => {
    if (label) onTitle(label);
  }, [label, onTitle]);

  // Archived or deleted while out here, from this window or the main one:
  // there is nothing left to show, so the window goes rather than sit empty.
  useEffect(() => {
    if (run) seen.current = true;
    else if (seen.current) onGone();
  }, [run, onGone]);

  // Approve lives in the main window (its merge window opens over the Agents
  // view and can deep-link into Source Control), so the request goes there.
  useEffect(() => {
    if (!approveRunId) return;
    setApproveRun(null);
    requestNavigate({ kind: "approve", projectId: target.projectId, runId: approveRunId });
  }, [approveRunId, setApproveRun, target.projectId]);

  // Notifications stay quiet about the agent on screen in the focused window
  // (see App.tsx). This window reports only when it gains or loses focus: an
  // unfocused popout reporting would tell the backend the app had lost focus
  // while the user was working in the main window.
  useEffect(() => {
    const report = () => setUiState(document.hasFocus(), onScreenRunId).catch(() => {});
    if (document.hasFocus()) report();
    window.addEventListener("focus", report);
    window.addEventListener("blur", report);
    return () => {
      window.removeEventListener("focus", report);
      window.removeEventListener("blur", report);
    };
  }, [onScreenRunId]);

  if (found === false) {
    return <div className="board empty">This agent is no longer in Agency.</div>;
  }
  if (!run) return <div className="app-loading"><span className="spinner" /></div>;
  return <AgentFocus solo />;
}

function PopoutFile({
  target,
  draft,
  collect,
}: {
  target: Extract<PopoutTarget, { kind: "file" }>;
  draft: string | null;
  collect: Collect;
}) {
  const editor = useRef<FileEditorHandle>(null);
  // The main window's unsaved edits, put where the editor looks for them on
  // mount. In an initializer so it lands before the editor's first read.
  useState(() => {
    if (draft !== null) stashBuffer(bufferKey(target.root, target.path), draft);
    return null;
  });
  collect.current = async () => {
    const d = editor.current?.unsaved() ?? null;
    // Going with the item: not to be stashed again here as the editor unmounts.
    editor.current?.discard();
    return d;
  };
  return (
    <div className="popout-editor">
      <FileEditor ref={editor} root={target.root} path={target.path} />
    </div>
  );
}

function PopoutNote({
  target,
  collect,
  onTitle,
}: {
  target: Extract<PopoutTarget, { kind: "note" }>;
  collect: Collect;
  onTitle: (t: string) => void;
}) {
  const { docsDir, index, refresh } = useDocs(target.projectId, true, target.root);
  const { cross } = useCrossRefs(true);
  const editor = useRef<DocsEditorHandle>(null);
  collect.current = async () => {
    await editor.current?.flush();
    return null;
  };

  const noteTitle = index?.docs.get(target.path)?.title;
  useEffect(() => {
    if (noteTitle) onTitle(noteTitle);
  }, [noteTitle, onTitle]);

  // A wikilink: the main window opens it (requestNavigate is forwarded there).
  const navigate = (to: string) => {
    const res = resolveTarget(index, cross, to);
    switch (res.kind) {
      case "note":
        requestNavigate({ kind: "note", projectId: target.projectId, path: res.path });
        return;
      case "issue":
        requestNavigate({ kind: "issue", projectId: res.ref.project.id, issueId: res.ref.issue.id });
        return;
      case "run":
        requestNavigate({ kind: "run", projectId: res.ref.project.id, runId: res.ref.run.id });
        return;
      case "unresolvedIssue":
        toastInfo(`No issue ${res.label} in any project.`);
        return;
      case "unresolvedRun":
        toastInfo("That run doesn't exist anymore.");
        return;
      case "unresolvedNote":
        toastInfo(`No note called "${to}". Create it from the main window.`);
    }
  };

  if (docsDir === undefined) return <div className="app-loading"><span className="spinner" /></div>;
  if (docsDir === null) return <div className="board empty">This project has no docs folder anymore.</div>;
  return (
    <div className="popout-editor">
      <DocsEditor
        ref={editor}
        root={target.root}
        docsDir={docsDir}
        path={target.path}
        diskText={index?.docs.get(target.path)?.text}
        index={index}
        cross={cross}
        onSaved={() => void refresh()}
        onNavigate={(to) => navigate(to)}
        onTagClick={() => {}}
        onFilter={() => {}}
        sideOpen={false}
      />
    </div>
  );
}
