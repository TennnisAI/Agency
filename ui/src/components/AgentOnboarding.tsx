import { CSSProperties, useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  CatalogEntry,
  InstallJob,
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
import { IS_MAC } from "../lib/platform";
import { jobFor, useToolInstalls, jobPending } from "../hooks/useToolInstalls";
import ModalBackdrop from "./ModalBackdrop";
import RepoSetupDialog from "./RepoSetupDialog";
import CloneDialog from "./CloneDialog";
import ToolRow from "./ToolRow";

// Stagger helper: each animated element carries its own entrance delay.
const rise = (ms: number) => ({ "--d": `${ms}ms` }) as CSSProperties;

type Step = "tools" | "agents" | "projects";

/** How often the agent tiles re-read PATH while the step is on screen. */
const CATALOG_POLL_MS = 3000;

// First run, in two or three moves: put the tools Agency shells out to in
// place when any are missing, pick the coding agents to enable, then point
// Agency at the repositories you work in. The hero column carries the copy
// and the primary action; the pane on the right holds whatever the step is
// asking for. Both remount on every step change so their entrance animations
// replay.
//
// Installs run in the background, here, with no terminal to visit: onboarding
// has no project yet and every in-app terminal belongs to one. A tile ticks
// itself when its command lands on PATH, whether Agency installed it or the
// user did in a shell of their own.
export default function AgentOnboarding({ onDone }: { onDone: () => void }) {
  const { tools, jobs, installTool, installAgent } = useToolInstalls(true);
  // The step list is fixed once the tools are known: a step that vanished the
  // moment git landed would pull the rail out from under the user.
  const [steps, setSteps] = useState<Step[] | null>(null);
  const [step, setStep] = useState<Step>("agents");
  const [catalog, setCatalog] = useState<CatalogEntry[] | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [customSaved, setCustomSaved] = useState(false);
  const [showCustom, setShowCustom] = useState(false);
  const [draft, setDraft] = useState({ name: "", command: "", args: "" });
  const [installFor, setInstallFor] = useState<string | null>(null);
  const [detailsFor, setDetailsFor] = useState<string | null>(null);
  // Agent installs waiting on npm: started once the Node tool lands.
  const [afterNode, setAfterNode] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);
  const copyTimer = useRef<number | undefined>(undefined);
  // Step three: repositories already known to Agency, plus the two add flows.
  const [projects, setProjects] = useState<Project[]>([]);
  const [setup, setSetup] = useState<{ path: string; name: string; readiness: RepoReadiness } | null>(null);
  const [cloning, setCloning] = useState(false);

  // Clear any pending copy-reset timer on unmount so it can't fire setCopied
  // after the component is gone.
  useEffect(() => () => window.clearTimeout(copyTimer.current), []);

  useEffect(() => {
    if (!tools || steps) return;
    const missing = tools.some((t) => !t.installed);
    const list: Step[] = missing ? ["tools", "agents", "projects"] : ["agents", "projects"];
    setSteps(list);
    setStep(list[0]);
  }, [tools, steps]);

  // The catalog, and the set of commands seen on PATH so far. A re-read that
  // finds a new one ticks its tile: that is the install, ours or the user's,
  // landing. Pre-selecting what is already there gives a machine with CLIs
  // installed a sensible default set.
  const seenInstalled = useRef<Set<string> | null>(null);
  const readCatalog = () => {
    listAgentCatalog()
      .then((entries) => {
        setCatalog(entries);
        const now = new Set(entries.filter((e) => e.installed).map((e) => e.id));
        if (seenInstalled.current === null) {
          setSelected(now);
        } else {
          const before = seenInstalled.current;
          const arrived = [...now].filter((id) => !before.has(id));
          if (arrived.length > 0) setSelected((s) => new Set([...s, ...arrived]));
        }
        seenInstalled.current = now;
      })
      .catch((e) => setError(String(e)));
  };
  useEffect(() => {
    readCatalog();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => {
    if (step !== "agents") return;
    const t = window.setInterval(readCatalog, CATALOG_POLL_MS);
    return () => window.clearInterval(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step]);
  // A finished install job is the moment to look, not three seconds later.
  const jobKeys = Object.values(jobs).map((j) => `${j.key}:${j.state}`).join(",");
  useEffect(() => {
    readCatalog();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [jobKeys]);

  const nodeReady = tools?.find((t) => t.id === "node")?.installed ?? false;
  const nodeJob = jobFor(jobs, "tool:node");

  // Start the agent installs that were waiting on npm once it is here.
  useEffect(() => {
    if (!nodeReady || afterNode.size === 0) return;
    for (const id of afterNode) {
      const cmd = INSTALL_COMMANDS[id];
      if (cmd) void installAgent(id, cmd);
    }
    setAfterNode(new Set());
  }, [nodeReady, afterNode, installAgent]);

  // ⌘↵ advances, matching the hint baked into the primary button.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key !== "Enter" || !(e.metaKey || e.ctrlKey)) return;
      // Whatever is layered on top owns the keyboard while it's open, and an
      // unsaved custom profile shouldn't be skipped past.
      if (installFor || detailsFor || setup || cloning || showCustom) return;
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
  // so a quit part-way through the last step doesn't hand back an unusable app.
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
    if (busy || !steps) return;
    if (step === "projects") {
      onDone();
      return;
    }
    if (step === "tools") {
      setError("");
      setStep("agents");
      return;
    }
    setBusy(true);
    const ok = await saveAgents();
    setBusy(false);
    if (!ok) return;
    listProjects().then(setProjects).catch(() => {});
    setStep("projects");
  }

  function back() {
    if (!steps) return;
    const i = steps.indexOf(step);
    if (i > 0) setStep(steps[i - 1]);
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

  // The install the confirm dialog runs. An npm line on a machine with no npm
  // installs Node first and queues the agent behind it; the tile shows both.
  function confirmInstall(id: string) {
    const cmd = INSTALL_COMMANDS[id];
    setInstallFor(null);
    if (!cmd) return;
    if (cmd.startsWith("npm ") && !nodeReady) {
      setAfterNode((s) => new Set([...s, id]));
      if (!jobPending(nodeJob)) void installTool("node");
      return;
    }
    void installAgent(id, cmd);
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
  const canAdvance = step !== "agents" || selected.size > 0 || customSaved;
  const stepList = steps ?? ["agents", "projects"];
  const stepIndex = Math.max(0, stepList.indexOf(step));
  const missingTools = tools?.filter((t) => !t.installed) ?? [];
  const nodePlanRuns = tools?.find((t) => t.id === "node")?.plan.kind === "run";

  // What an agent tile's footer says about its install, if one is in flight
  // or has run: the tool it waits on, then its own job.
  function tileInstall(id: string): { text: string; state: "busy" | "bad" } | null {
    if (afterNode.has(id)) {
      return nodeJob?.state === "failed"
        ? { text: "Node.js install failed", state: "bad" }
        : { text: "Installing Node.js first…", state: "busy" };
    }
    const job = jobFor(jobs, `agent:${id}`);
    if (!job) return null;
    if (job.state === "queued") return { text: "Waiting…", state: "busy" };
    if (job.state === "running") return { text: "Installing…", state: "busy" };
    if (job.state === "failed") return { text: "Install failed", state: "bad" };
    return null;
  }

  const titles: Record<Step, JSX.Element> = {
    tools: (
      <>
        <h1 className="onboard-title" style={rise(60)}>
          <span className="dim">Put the</span>{" "}
          <span className="onboard-key">
            <span className="onboard-glyph" aria-hidden>⚙︎</span>tools
          </span>{" "}
          <span className="dim">in place.</span>
        </h1>
        <p className="onboard-sub" style={rise(130)}>
          Agency shells out to a few command-line tools. Install the ones this machine is
          missing here, or skip ahead and come back to them later.
        </p>
      </>
    ),
    agents: (
      <>
        <h1 className="onboard-title" style={rise(60)}>
          <span className="dim">Pick the</span>{" "}
          <span className="onboard-key">
            <span className="onboard-glyph" aria-hidden>▦</span>agents
          </span>{" "}
          <span className="dim">you work with.</span>
        </h1>
        <p className="onboard-sub" style={rise(130)}>
          Agency preselects whatever it finds on your PATH, and ticks an agent the moment it is
          installed. You can enable, add, or edit agents anytime in Settings.
        </p>
      </>
    ),
    projects: (
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
    ),
  };

  // Nothing until the tool check answers (a few PATH lookups): the first step
  // depends on it, and a hero that changed its mind a moment after appearing
  // would read as a glitch.
  if (!steps) return <div className="onboard" />;

  return (
    <div className="onboard">
      <div className="onboard-glow" aria-hidden />
      <div className="onboard-inner">
        <section className="onboard-hero" key={`hero-${step}`}>
          <div className="onboard-rail" style={rise(0)}>
            <div className="onboard-steps" aria-hidden>
              {stepList.map((s, i) => (
                <span
                  key={s}
                  className={`onboard-seg${s === step ? " is-on" : i < stepIndex ? " is-done" : ""}`}
                />
              ))}
            </div>
            <span className="onboard-step-label">
              Step {stepIndex + 1} of {stepList.length}
            </span>
          </div>

          {titles[step]}

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
              <span>{busy ? "Saving…" : step === "projects" ? "Finish" : "Continue"}</span>
              <span className="onboard-keys" aria-hidden>
                <span className="onboard-cap">{IS_MAC ? "⌘" : "Ctrl"}</span>
                <span className="onboard-cap">{IS_MAC ? "↵" : "Enter"}</span>
              </span>
            </button>
            {stepIndex > 0 && (
              <button type="button" className="onboard-back" onClick={back}>
                Back
              </button>
            )}
          </div>

          {step === "tools" && (
            <p className="onboard-note" style={rise(260)}>
              {missingTools.some((t) => t.id === "git")
                ? "Without git, projects are plain folders: agents work in them directly, with no branches and nothing to merge."
                : "Everything Agency needs is here."}
            </p>
          )}
          {step === "projects" && (
            <p className="onboard-note" style={rise(260)}>
              {projects.length === 0
                ? "Nothing added yet. Projects can be added anytime from the sidebar."
                : "You can add more anytime from the sidebar."}
            </p>
          )}
        </section>

        <section className="onboard-pane" key={`pane-${step}`}>
          {step === "tools" ? (
            <>
              <div className="onboard-pane-head" style={rise(40)}>
                <span>Command-line tools</span>
                <span className="onboard-count">
                  {missingTools.length > 0 ? `${missingTools.length} missing` : "All found"}
                </span>
              </div>
              <div className="onboard-tools">
                {(tools ?? []).map((t, i) => (
                  <div key={t.id} style={rise(60 + i * 45)}>
                    <ToolRow
                      tool={t}
                      job={jobFor(jobs, `tool:${t.id}`)}
                      onInstall={() => void installTool(t.id)}
                    />
                  </div>
                ))}
              </div>
            </>
          ) : step === "agents" ? (
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
                      const progress = entry.installed ? null : tileInstall(entry.id);
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
                            ) : progress ? (
                              <span className={`onboard-state is-${progress.state}`}>
                                {progress.state === "busy" && <span className="spinner small" />}
                                {progress.text}
                              </span>
                            ) : (
                              <span className="onboard-state">Not installed</span>
                            )}
                            {entry.servesWebUi && (
                              <span
                                className="onboard-state"
                                title="This agent's interactive surface is a browser app it serves on this machine only. Agency shows it beside the run's log pane."
                              >Browser GUI</span>
                            )}
                            {!entry.installed && install && progress?.state !== "busy" && (
                              <button
                                type="button"
                                className="onboard-install"
                                onClick={(e) => {
                                  e.stopPropagation();
                                  if (progress?.state === "bad") setDetailsFor(entry.id);
                                  else setInstallFor(entry.id);
                                }}
                              >
                                {progress?.state === "bad" ? "Details…" : "Install…"}
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
              {installCmd ? (
                <>
                  <p className="install-lede">
                    Agency runs this in the background and ticks{" "}
                    <code>{catalog?.find((e) => e.id === installFor)?.command}</code> here when it lands.
                    You can keep going meanwhile.
                  </p>
                  <pre className="install-cmd">{installCmd}</pre>
                  {installCmd.startsWith("npm ") && !nodeReady && (
                    <p className="modal-note">
                      {nodePlanRuns
                        ? "This needs npm, which isn't installed yet. Agency installs Node.js and npm first, then this."
                        : "This needs npm, which isn't installed yet and has to be installed by hand on this machine. Go back to the tools step for the details."}
                    </p>
                  )}
                </>
              ) : (
                <p>Install it manually and point the profile at the right command in Settings.</p>
              )}
            </div>
            <div className="modal-foot">
              <button className="btn-secondary" onClick={() => setInstallFor(null)}>Cancel</button>
              {installCmd && (
                <button className="btn-secondary" onClick={() => copyInstall(installFor)}>
                  {copied ? "Copied" : "Copy command"}
                </button>
              )}
              {installCmd && (!installCmd.startsWith("npm ") || nodeReady || nodePlanRuns) && (
                <button className="btn-primary" autoFocus onClick={() => confirmInstall(installFor)}>
                  Install
                </button>
              )}
            </div>
          </div>
        </ModalBackdrop>
      )}

      {detailsFor && (
        <InstallDetails
          agent={detailsFor}
          job={afterNode.has(detailsFor) ? nodeJob : jobFor(jobs, `agent:${detailsFor}`)}
          onRetry={() => {
            setDetailsFor(null);
            setInstallFor(detailsFor);
          }}
          onCopy={() => copyInstall(detailsFor)}
          copied={copied}
          onClose={() => setDetailsFor(null)}
        />
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

// What a failed install printed, with the two ways forward: run it again, or
// take the line to a terminal of your own.
function InstallDetails({
  agent,
  job,
  onRetry,
  onCopy,
  copied,
  onClose,
}: {
  agent: string;
  job: InstallJob | null;
  onRetry: () => void;
  onCopy: () => void;
  copied: boolean;
  onClose: () => void;
}) {
  return (
    <ModalBackdrop onBackdropClick={onClose}>
      <div
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-label={`${agentLabel(agent)} install failed`}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h3>{agentLabel(agent)} didn't install</h3>
          <button className="modal-x" onClick={onClose}>✕</button>
        </div>
        <div className="modal-body">
          <p className="install-lede">
            The installer exited{job?.exitCode != null ? ` with code ${job.exitCode}` : ""}. This is the
            end of what it printed:
          </p>
          <pre className="tool-output">{job?.output || "(nothing)"}</pre>
          <pre className="install-cmd">{INSTALL_COMMANDS[agent]}</pre>
        </div>
        <div className="modal-foot">
          <button className="btn-secondary" onClick={onClose}>Close</button>
          <button className="btn-secondary" onClick={onCopy}>{copied ? "Copied" : "Copy command"}</button>
          <button className="btn-primary" onClick={onRetry}>Try again</button>
        </div>
      </div>
    </ModalBackdrop>
  );
}
