import { useRef, useState } from "react";
import { QueuedMessage, RunInfo, cancelQueuedMessage, listQueuedMessages } from "../api";
import { useRuns } from "../store/runs";
import { toastError, toastInfo } from "../lib/toast";
import { useDismissOnResize } from "../hooks/useDismissOnResize";

// The marker a run wears while Agency still owes its agent a message.
//
// The three senders (review comments, failing checks, a conflicted merge) each
// say in place whether the text went in or is waiting, but that only covers the
// moment of sending: close the merge window and nothing anywhere said a prompt
// was still held, so a message waiting behind a long turn looked exactly like
// one that was never sent.
//
// Opening it lists what is waiting and offers to drop it. The queue is the only
// thing in Agency that types into a session with nobody watching, so cancelling
// has to be possible.
export default function QueuedMarker({ run }: { run: RunInfo }) {
  const { refreshRuns } = useRuns();
  const [open, setOpen] = useState(false);
  const [items, setItems] = useState<QueuedMessage[]>([]);
  const [coords, setCoords] = useState<{ top: number; left: number }>();
  const btnRef = useRef<HTMLButtonElement>(null);

  // The popover is placed once at viewport coordinates (like OverflowMenu), so
  // an ancestor's overflow can't clip it; a resize moves the chip, not it.
  useDismissOnResize(open, () => setOpen(false));

  if (run.queuedMessages < 1) return null;

  const load = async () => {
    try {
      setItems(await listQueuedMessages(run.id));
    } catch {
      setItems([]);
    }
  };

  const toggle = async () => {
    if (open) {
      setOpen(false);
      return;
    }
    const r = btnRef.current?.getBoundingClientRect();
    if (r) setCoords({ top: r.bottom + 5, left: Math.max(8, Math.min(r.left, window.innerWidth - 340)) });
    await load();
    setOpen(true);
  };

  // The row clamps the prompt to three lines, so what is waiting is
  // recognisable but not readable: the merge-conflict prompt names every
  // unmerged file and runs well past that. This hands over the whole of it, to
  // paste into the agent by hand rather than waiting for the queue to reach it.
  const copy = async (m: QueuedMessage) => {
    try {
      await navigator.clipboard.writeText(m.text);
      toastInfo("Prompt copied");
    } catch (e) {
      toastError(e, "Couldn't copy the prompt");
    }
  };

  const drop = async (m: QueuedMessage) => {
    let dropped = false;
    try {
      dropped = await cancelQueuedMessage(m.sessionId, m.text);
    } catch (e) {
      toastError(e, "Couldn't drop the message");
      return;
    }
    // The queue drains on the notifier's tick, so a message can go in between
    // this list being drawn and the click landing. A toast rather than a line
    // in the popover: dropping the last one takes the marker (and the popover
    // with it) off the run, and this is exactly the case the user has to see.
    if (!dropped) toastInfo("That one went into the agent before it could be dropped.");
    await load();
    await refreshRuns();
  };

  const n = run.queuedMessages;
  return (
    // Swallows clicks so the marker works the same on a tile, where the card
    // behind it opens the run.
    <span className="queued-wrap" onClick={(e) => e.stopPropagation()}>
      <button
        ref={btnRef}
        className={`queued-chip${open ? " on" : ""}`}
        title={`Agency is holding ${n} message${n === 1 ? "" : "s"} for this agent and types ${n === 1 ? "it" : "them"} in once the agent is free. Click to see what is waiting.`}
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={toggle}
      >
        <span aria-hidden>⧗</span> {n} queued
      </button>
      {open && (
        <>
          <div className="agent-menu-backdrop" onClick={() => setOpen(false)} />
          <div className="queued-pop" role="dialog" style={{ position: "fixed", ...coords }}>
            <div className="queued-pop-head">
              Waiting to go into this agent, as soon as it is between turns with an empty prompt
              line.
            </div>
            {items.length === 0 && <div className="queued-pop-empty">Nothing is waiting now.</div>}
            {items.map((m, i) => (
              <div className="queued-row" key={`${m.sessionId}-${i}`}>
                <div className="queued-row-text">
                  <span className="queued-origin">{m.origin}</span>
                  <span className="queued-body">{m.text}</span>
                </div>
                <div className="queued-actions">
                  <button
                    className="queued-act queued-copy"
                    title="Copy the whole prompt, to paste into the agent yourself"
                    onClick={() => copy(m)}
                  >Copy</button>
                  <button
                    className="queued-act queued-drop"
                    title="Drop this message. Nothing is typed into the agent."
                    onClick={() => drop(m)}
                  >Drop</button>
                </div>
              </div>
            ))}
          </div>
        </>
      )}
    </span>
  );
}
