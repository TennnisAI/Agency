import { useEffect, useRef, useState } from "react";
import { AgentModelInfo, probeAgentModels } from "../api";
import { filterModels, modelIdError, modelOptions } from "../agents";
import { useDismissOnResize } from "../hooks/useDismissOnResize";

// Probed lists, kept for as long as the window is open so reopening a picker
// doesn't flash a spinner at what it already showed. The Rust side caches for
// the app session; this caches for the webview's, and a reload just asks again
// and is answered from that cache.
const probed = new Map<string, string[]>();

// Model picker for a single agent: the vendor's stable aliases, whatever has
// been used with this agent before, whatever the agent's own CLI says it has,
// and a field for anything else.
//
// The built-in list is deliberately short. Every one of these CLIs can reach
// models Agency has never heard of, and a hardcoded catalogue of dated model
// names would be wrong within a release, so the ids shipped here are only the
// aliases the vendor keeps pointed at the current model.
//
// That left the CLIs with no stable aliases (Codex, Cursor, opencode, pi, and
// the rest) offering nothing at all, so opening this menu now also asks the
// ones that can answer what they have: opencode, pi and Cursor today
// (AGE-117). It asks on open and not before, because the command runs the
// agent's binary and takes about a second. The typed field stays either way, as
// a probe can fail and a CLI can run a model it doesn't list.
//
// Same trigger-plus-portal shape as BranchSelect, for the same reason: a native
// <select> is sized by its widest option and can't hold a text field anyway.
export default function ModelSelect({
  info,
  value,
  onChange,
  compact = false,
}: {
  // Undefined while the model list is still loading, and carrying
  // `supported: false` for an agent whose CLI takes no model flag. Either way
  // the control renders nothing (see below).
  info: AgentModelInfo | undefined;
  value: string | null;
  onChange: (model: string | null) => void;
  // Chip form, for a row that already names the agent.
  compact?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const [typed, setTyped] = useState("");
  const [error, setError] = useState("");
  const [listed, setListed] = useState<string[]>([]);
  const [listing, setListing] = useState(false);
  const [listError, setListError] = useState("");
  const [coords, setCoords] = useState<{
    top?: number; bottom?: number; left?: number; right?: number; minWidth: number;
  }>({ minWidth: 0 });
  const btnRef = useRef<HTMLButtonElement>(null);

  // Escape closes this popup only, leaving the menu or dialog it opened from
  // standing — the same thing a click on the backdrop does.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      setOpen(false);
      btnRef.current?.focus();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [open]);

  // The popup is placed against the trigger's rect, measured once at open.
  useDismissOnResize(open, () => setOpen(false));

  // Ask the agent what it has, once per session per agent. A failure is shown
  // and not remembered: the usual reason one fails is that the agent isn't
  // logged in yet, which is fixed in another window while this stays open.
  const agent = info?.agent;
  const listCommand = info?.listCommand;
  useEffect(() => {
    if (!open || !agent || !listCommand) return;
    const cached = probed.get(agent);
    setListed(cached ?? []);
    setListError("");
    if (cached) return;
    let live = true;
    setListing(true);
    probeAgentModels(agent)
      .then((models) => {
        probed.set(agent, models);
        if (live) setListed(models);
      })
      .catch((e) => { if (live) setListError(String(e)); })
      .finally(() => { if (live) setListing(false); });
    return () => { live = false; };
  }, [open, agent, listCommand]);

  // Nothing to pick until we know this agent can be told a model at all: an
  // agent whose CLI takes no model flag gets no control, and neither does one
  // whose entry hasn't loaded, so a failed load leaves runs on their defaults
  // rather than offering a choice that could not be applied.
  if (!info?.supported) return null;

  const options = modelOptions(info.suggested, info.recent, listed);
  // The field filters as it's typed: a probed list runs past 200 ids for some
  // agents, and scrolling that is not picking from it.
  const shown = filterModels(options, typed);

  const toggle = () => {
    setOpen((o) => {
      const next = !o;
      const r = btnRef.current?.getBoundingClientRect();
      if (next) {
        setTyped("");
        setError("");
      }
      if (next && r) {
        const MENU_W = Math.max(r.width, 240);
        // A menu that is about to ask for a list has to be placed for the size
        // it will be, not the size it opens at: the answer arrives a second
        // later and would otherwise grow the menu straight off the bottom of
        // the screen.
        const height = info.listCommand ? 320 : Math.min(320, (options.length + 3) * 30 + 60);
        const fitsRight = r.left + MENU_W <= window.innerWidth - 8;
        const fitsBelow = r.bottom + 4 + height <= window.innerHeight - 8;
        setCoords({
          ...(fitsBelow ? { top: r.bottom + 4 } : { bottom: window.innerHeight - r.top + 4 }),
          ...(fitsRight ? { left: r.left } : { right: window.innerWidth - r.right }),
          minWidth: Math.max(r.width, 240),
        });
      }
      return next;
    });
  };

  const pick = (model: string | null) => {
    setOpen(false);
    onChange(model);
  };

  // The field is a filter as well as an entry box, so Enter (and Use) resolves
  // in that order: an exact match picks that model, a filter narrowed to one
  // picks the one left, and anything else is taken literally, which is the only
  // way to reach a model the CLI didn't list.
  const applyTyped = () => {
    const model = typed.trim();
    if (model && shown.includes(model)) return pick(model);
    if (model && shown.length === 1) return pick(shown[0]);
    const problem = modelIdError(typed);
    if (problem) return setError(problem);
    pick(model);
  };

  const { minWidth, ...pos } = coords;
  const label = value ?? "default";

  // One line under the field, in the order that matters: what you just typed is
  // wrong, then why the agent couldn't be asked, then that it is being asked,
  // then that the filter has hidden everything (which otherwise looks like an
  // agent with no models), then what answered. An agent with no listing command
  // and nothing typed has nothing to say here.
  const note = error ? (
    error
  ) : listError ? (
    listError
  ) : listing ? (
    <>Asking <code>{info.listCommand}</code>…</>
  ) : typed.trim() && shown.length === 0 && options.length > 0 ? (
    <>No listed model matches; Use takes it anyway.</>
  ) : listed.length > 0 ? (
    <>{listed.length} models from <code>{info.listCommand}</code></>
  ) : null;

  return (
    <>
      <button
        ref={btnRef}
        type="button"
        className={compact ? "model-chip" : "branch-select"}
        title={value ? `Model: ${value}` : "Model: the agent's own default"}
        onClick={(e) => { e.stopPropagation(); toggle(); }}
      >
        <span className="branch-select-name">{label}</span>
        <span className="branch-select-caret">▾</span>
      </button>
      {open && (
        <>
          <div className="branch-menu-backdrop" onClick={(e) => { e.stopPropagation(); setOpen(false); }} />
          <div className="agent-menu model-menu" style={{ position: "fixed", minWidth, ...pos }}>
            <button
              type="button"
              className={value === null ? "on" : ""}
              onClick={(e) => { e.stopPropagation(); pick(null); }}
            >
              <span className="branch-select-name">Agent's default</span>
              {value === null && <span className="branch-menu-tick">✓</span>}
            </button>
            {shown.length > 0 && <div className="agent-menu-sep" />}
            {shown.length > 0 && (
              <div className="model-options">
                {shown.map((m) => (
                  <button
                    key={m}
                    type="button"
                    title={m}
                    className={m === value ? "on" : ""}
                    // Keep the current model in view when a probed list is long
                    // enough to scroll, so opening the menu shows where you are.
                    ref={m === value ? (el) => el?.scrollIntoView({ block: "nearest" }) : undefined}
                    onClick={(e) => { e.stopPropagation(); pick(m); }}
                  >
                    <span className="branch-select-name">{m}</span>
                    {m === value && <span className="branch-menu-tick">✓</span>}
                  </button>
                ))}
              </div>
            )}
            <div className="agent-menu-sep" />
            <div className="model-other" onClick={(e) => e.stopPropagation()}>
              <input
                className="settings-input"
                placeholder={options.length > 0 ? "Filter, or type a model id…" : "Model id…"}
                value={typed}
                spellCheck={false}
                autoComplete="off"
                onChange={(e) => { setTyped(e.target.value); setError(""); }}
                onKeyDown={(e) => {
                  if (e.key !== "Enter") return;
                  e.preventDefault();
                  applyTyped();
                }}
              />
              <button type="button" className="model-use" disabled={!typed.trim()} onClick={applyTyped}>
                Use
              </button>
            </div>
            {note && (
              <div className={`model-note${error || listError ? " model-error" : ""}`}>{note}</div>
            )}
          </div>
        </>
      )}
    </>
  );
}
