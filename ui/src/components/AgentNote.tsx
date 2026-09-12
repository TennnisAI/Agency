import { RunInfo } from "../api";
import { agentNote } from "../lib/runstate";

// The line the agent wrote about itself with set_status (AGE-208), under the
// card's branch and diff. The text is the agent's own, so it goes in as a
// React text child and nothing else: no markup, no link detection, no title
// built from it. It sits under the run's state rather than in its place.
export default function AgentNote({ run }: { run: RunInfo }) {
  const note = agentNote(run);
  if (!note) return null;
  return (
    <div className={`tile-note${note.stale ? " stale" : ""}`} title={note.title}>
      {note.text}
    </div>
  );
}
