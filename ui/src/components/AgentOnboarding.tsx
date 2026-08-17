import { CSSProperties, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  CatalogEntry,
  Project,
  RepoReadiness,
  addProject,
  completeAgentOnboarding,
  inspectRepo,
  listAgentCatalog,
  listProfiles,
  listProjects,
  saveProfile,
} from "../api";
import { INSTALL_COMMANDS, agentColor, agentLabel } from "../agents";
import { toastError } from "../lib/toast";
import ModalBackdrop from "./ModalBackdrop";
import RepoSetupDialog from "./RepoSetupDialog";
import CloneDialog from "./CloneDialog";

// Stagger helper: each animated element carries its own entrance delay.
const rise = (ms: number) => ({ "--d": `${ms}ms` }) as CSSProperties;

type Step = "agents" | "projects";
const STEPS: Step[] = ["agents", "projects"];

// First run, in two moves: pick the coding agents to enable, then point Agency
// at the repositories you work in. The hero column carries the copy and the
// primary action; the pane on the right holds whatever the step is asking for.
// Both remount on every step change so their entrance animations replay.
export default function AgentOnboarding({ onDone }: { onDone: () => void }) {
  const [step, setStep] = useState<Step>("agents");
  const [catalog, setCatalog] = useState<CatalogEntry[] | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [customSaved, setCustomSaved] = useState(false);
  const [showCustom, setShowCustom] = useState(false);
  const [draft, setDraft] = useState({ name: "", command: "", args: "" });
  const [installFor, setInstallFor] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);
  const copyTimer = useRef<number | undefined>(undefined);
  // Step two: repositories already known to Agency, plus the two add flows.
  const [projects, setProjects] = useState<Project[]>([]);
  const [setup, setSetup] = useState<{ path: string; name: string; readiness: RepoReadiness } | null>(null);
  const [cloning, setCloning] = useState(false);

  // Clear any pending copy-reset timer on unmount so it can't fire setCopied
  // after the component is gone.
  useEffect(() => () => window.clearTimeout(copyTimer.current), []);

  useEffect(() => {
    listAgentCatalog()
      .then((entries) => {
        setCatalog(entries);
        // Pre-select agents already on PATH so a machine with CLIs installed
        // lands on a sensible default set.
        setSelected(new Set(entries.filter((e) => e.installed).map((e) => e.id)));
      })
      .catch((e) => setError(String(e)));
  }, []);

  // ⌘↵ advances, matching the hint baked into the primary button.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key !== "Enter" || !(e.metaKey || e.ctrlKey)) return;
      // Whatever is layered on top owns the keyboard while it's open, and an
      // unsaved custom profile shouldn't be skipped past.
      if (installFor || setup || cloning || showCustom) return;
      e.preventDefault();
      void advance();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  function toggle(id: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  async function saveCustom() {
    const name = draft.name.trim();
    const command = draft.command.trim();
    if (!name || !command) return;
    const args = draft.args.trim() ? draft.args.trim().split(/\s+/) : [];
    try {
      await saveProfile({
        name,
        command,
        args,
        env: [],
        resume_args: null,
        loop_args: null,
      });
      setCustomSaved(true);
      setShowCustom(false);
      setDraft({ name: "", command: "", args: "" });
    } catch (e) {
      toastError(e, "Couldn't save custom profile");
    }
  }

  // Leaving the agents step writes the selection and clears the first-run flag,
  // so a quit part-way through step two doesn't hand back an unusable app.
  // Re-entry is harmless: enabling a catalog agent is idempotent.
  async function saveAgents(): Promise<boolean> {
    const ids = [...selected];
    if (ids.length === 0 && !customSaved) {
      // Custom may have been saved before this render — re-check profiles.
      try {
        const profiles = await listProfiles();
        if (profiles.length === 0) {
          setError("Select at least one agent, or add a custom profile.");
          return false;
        }
      } catch {
        setError("Select at least one agent, or add a custom profile.");
        return false;
      }
    }
    setError("");
    try {
      await completeAgentOnboarding(ids);
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    }
  }

  async function advance() {
    if (busy) return;
    if (step === "projects") {
      onDone();
      return;
    }
    setBusy(true);
    const ok = await saveAgents();
    setBusy(false);
    if (!ok) return;
    listProjects().then(setProjects).catch(() => {});
    setStep("projects");
  }

  async function copyInstall(id: string) {
    const cmd = INSTALL_COMMANDS[id];
    if (!cmd) return;
    try {
      await navigator.clipboard.writeText(cmd);
      setCopied(true);
      window.clearTimeout(copyTimer.current);
      copyTimer.current = window.setTimeout(() => setCopied(false), 1500);
    } catch (e) {
      toastError(e, "Couldn't copy to clipboard");
    }
  }

  // Same add-project flow as the Projects pane: pick a directory, add it
  // straight away when the repo is ready, otherwise route through setup.
  async function addFolder() {
    const sel = await open({ directory: true, multiple: false });
    if (typeof sel !== "string") return;
    const name = sel.split("/").filter(Boolean).pop() ?? sel;
    setError("");
    try {
      const r = await inspectRepo(sel);
      if (r.state !== "ready" || r.dirty) {
        setSetup({ path: sel, name, readiness: r });
        return;
      }
      const added = await addProject(name, sel);
      setProjects((p) => [...p, added]);
    } catch (e) {
      setError(String(e));
    }
  }

  async function finishSetup() {
    if (!setup) return;
    try {
      const added = await addProject(setup.name, setup.path);
      setProjects((p) => [...p, added]);
    } catch (e) {
      setError(String(e));
    }
    setSetup(null);
  }

  // A freshly cloned repo has commits and a clean tree — add it as-is.
  async function handleCloned(path: string) {
    setCloning(false);
    const name = path.split("/").filter(Boolean).pop() ?? path;
    setError("");
    try {
      const added = await addProject(name, path);
      setProjects((p) => [...p, added]);
    } catch (e) {
      setError(String(e));
    }
  }

  const installCmd = installFor ? INSTALL_COMMANDS[installFor] : null;
  const canAdvance = step === "projects" || selected.size > 0 || customSaved;

  return (
    <div className="onboard">
      <div className="onboard-glow" aria-hidden />
      <div className="onboard-inner">
        <section className="onboard-hero" key={`hero-${step}`}>
          <div className="onboard-rail" style={rise(0)}>
            <div className="onboard-steps" aria-hidden>
              {STEPS.map((s, i) => (
                <span
                  key={s}
                  className={`onboard-seg${s === step ? " is-on" : i < STEPS.indexOf(step) ? " is-done" : ""}`}
                />
              ))}
            </div>
            <span className="onboard-step-label">
              Step {STEPS.indexOf(step) + 1} of {STEPS.length}
            </span>
          </div>

          {step === "agents" ? (
            <>
              <h1 className="onboard-title" style={rise(60)}>
                <span className="dim">Pick the</span>{" "}
                <span className="onboard-key">
                  <span className="onboard-glyph" aria-hidden>▦</span>agents
                </span>{" "}
                <span className="dim">you work with.</span>
              </h1>
              <p className="onboard-sub" style={rise(130)}>
                Agency preselects whatever it finds on your PATH. You can enable, add, or edit
                agents anytime in Settings.
              </p>
            </>
          ) : (
            <>
              <h1 className="onboard-title" style={rise(60)}>
                <span className="dim">Point Agency at your</span>{" "}
                <span className="onboard-key">
                  <span className="onboard-glyph" aria-hidden>▣</span>projects
                </span>
                <span className="dim">.</span>
              </h1>
              <p className="onboard-sub" style={rise(130)}>
                A project is a local git repository. Add the ones you're working in now; the rest
                can follow later.
              </p>
            </>
          )}

          {error && (
            <div className="git-error onboard-error" style={rise(160)}>
              {error}
            </div>
          )}

          <div className="onboard-cta" style={rise(200)}>
            <button
              type="button"
              className="onboard-go"
              disabled={!canAdvance || busy}
              onClick={() => void advance()}
            >
              <span>{busy ? "Saving…" : step === "agents" ? "Continue" : "Finish"}</span>
              <span className="onboard-keys" aria-hidden>
                <span className="onboard-cap">⌘</span>
                <span className="onboard-cap">↵</span>
              </span>
            </button>
            {step === "projects" && (
              <button type="button" className="onboard-back" onClick={() => setStep("agents")}>
                Back
              </button>
            )}
          </div>

          {step === "projects" && (
            <p className="onboard-note" style={rise(260)}>
              {projects.length === 0
                ? "Nothing added yet. Projects can be added anytime from the sidebar."
                : "You can add more anytime from the sidebar."}
            </p>
          )}
        </section>

        <section className="onboard-pane" key={`pane-${step}`}>
          {step === "agents" ? (
            <>
              <div className="onboard-pane-head" style={rise(40)}>
                <span>Coding agents</span>
                <span className="onboard-count">
                  {selected.size > 0 ? `${selected.size} selected` : "None selected"}
                </span>
              </div>
              <div className="onboard-grid">
                {!catalog
                  ? Array.from({ length: 6 }, (_, i) => (
                      <div
                        key={i}
                        className="onboard-tile is-skeleton"
                        style={rise(60 + i * 40)}
                        aria-hidden
                      />
                    ))
                  : catalog.map((entry, i) => {
                      const checked = selected.has(entry.id);
                      const install = INSTALL_COMMANDS[entry.id];
                      return (
                        <div
                          key={entry.id}
                          className={`onboard-tile${checked ? " is-on" : ""}`}
                          style={rise(60 + i * 35)}
                          role="checkbox"
                          aria-checked={checked}
                          tabIndex={0}
                          onClick={() => toggle(entry.id)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter" || e.key === " ") {
                              e.preventDefault();
                              toggle(entry.id);
                            }
                          }}
                        >
                          <div className="onboard-tile-top">
                            <span className="agent-dot" style={{ background: agentColor(entry.id) }} />
                            <span className="onboard-tile-name">{agentLabel(entry.id)}</span>
                            <span className="onboard-tick" aria-hidden>
                              {checked ? "✓" : ""}
                            </span>
                          </div>
                          <code className="onboard-tile-cmd">{entry.command}</code>
                          <div className="onboard-tile-foot">
                            {entry.installed ? (
                              <span className="onboard-state is-ok">On PATH</span>
                            ) : (
                              <span className="onboard-state">Not installed</span>
                            )}
                            {!entry.installed && install && (
                              <button
                                type="button"
                                className="onboard-install"
                                onClick={(e) => {
                                  e.stopPropagation();
                                  setInstallFor(entry.id);
                                }}
                              >
                                Install…
                              </button>
                            )}
                          </div>
                        </div>
                      );
                    })}

                {catalog && !showCustom && (
                  <button
                    type="button"
                    className="onboard-tile onboard-tile-add"
                    style={rise(60 + catalog.length * 35)}
                    onClick={() => setShowCustom(true)}
                  >
                    <span className="onboard-plus" aria-hidden>+</span>
                    <span>Custom agent</span>
                  </button>
                )}
              </div>

              {showCustom && (
                <div className="onboard-custom">
                  <div className="settings-form-label">Custom agent</div>
                  <input
                    className="settings-input"
                    autoFocus
                    placeholder="name"
                    value={draft.name}
                    onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                  />
                  <input
                    className="settings-input"
                    placeholder="command"
                    value={draft.command}
                    onChange={(e) => setDraft({ ...draft, command: e.target.value })}
                  />
                  <input
                    className="settings-input"
                    placeholder="args (optional, use {{prompt}})"
                    value={draft.args}
                    onChange={(e) => setDraft({ ...draft, args: e.target.value })}
                  />
                  <div className="row-actions">
                    <button type="button" className="btn-primary" onClick={saveCustom}>
                      Save custom
                    </button>
                    <button type="button" className="btn-secondary" onClick={() => setShowCustom(false)}>
                      Cancel
                    </button>
                  </div>
                </div>
              )}

              {customSaved && (
                <p className="onboard-custom-ok">Custom profile saved. It joins your agent list.</p>
              )}
            </>
          ) : (
            <>
              <div className="onboard-pane-head" style={rise(40)}>
                <span>Repositories</span>
                <span className="onboard-count">
                  {projects.length > 0 ? `${projects.length} added` : "Optional"}
                </span>
              </div>
              <div className="onboard-chips">
                {projects.map((p, i) => (
                  <span
                    key={p.id}
                    className="onboard-chip"
                    style={rise(60 + i * 35)}
                    title={p.repo_path}
                  >
                    <span className="onboard-chip-glyph" aria-hidden>
                      {p.kind === "workspace" ? "◈" : "▢"}
                    </span>
                    {p.name}
                  </span>
                ))}
                <button
                  type="button"
                  className="onboard-chip onboard-chip-add"
                  style={rise(60 + projects.length * 35)}
                  onClick={() => void addFolder()}
                >
                  <span className="onboard-plus" aria-hidden>+</span>
                  Add a folder
                </button>
                <button
                  type="button"
                  className="onboard-chip onboard-chip-add"
                  style={rise(95 + projects.length * 35)}
                  onClick={() => setCloning(true)}
                >
                  <span className="onboard-plus" aria-hidden>↓</span>
                  Clone from a URL
                </button>
              </div>
              <p className="onboard-pane-note" style={rise(220)}>
                A folder that isn't a git repository yet gets set up on the spot.
              </p>
            </>
          )}
        </section>
      </div>

      {installFor && (
        <ModalBackdrop onBackdropClick={() => setInstallFor(null)}>
          <div
            className="modal confirm"
            role="dialog"
            aria-modal="true"
            aria-label={`Install ${agentLabel(installFor)}`}
            onClick={(e) => e.stopPropagation()}
          >
            <div className="modal-head">
              <h3>Install {agentLabel(installFor)}</h3>
              <button className="modal-x" onClick={() => setInstallFor(null)}>✕</button>
            </div>
            <div className="modal-body">
              <p className="install-lede">
                Run this in a terminal to install <code>{catalog?.find((e) => e.id === installFor)?.command}</code>,
                then come back and continue. After you add a project, Agency can also
                run installs for you when spawning an agent.
              </p>
              {installCmd ? (
                <pre className="install-cmd">{installCmd}</pre>
              ) : (
                <p>Install it manually and point the profile at the right command in Settings.</p>
              )}
            </div>
            <div className="modal-foot">
              <button className="btn-secondary" onClick={() => setInstallFor(null)}>Close</button>
              {installCmd && (
                <button className="btn-primary" onClick={() => copyInstall(installFor)}>
                  {copied ? "Copied" : "Copy command"}
                </button>
              )}
            </div>
          </div>
        </ModalBackdrop>
      )}

      {setup && (
        <RepoSetupDialog
          readiness={setup.readiness}
          context="add"
          repoPath={setup.path}
          onResolved={finishSetup}
          onCancel={() => setSetup(null)}
        />
      )}
      {cloning && (
        <CloneDialog onCloned={handleCloned} onCancel={() => setCloning(false)} />
      )}
    </div>
  );
}
