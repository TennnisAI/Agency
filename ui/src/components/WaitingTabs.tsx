import { RunInfo } from "../api";
import { RunTab, waitingElsewhere, waitingElsewhereLabel } from "../lib/runTabs";

/**
 * The tile foot's word for tabs waiting on you that the tile's own dot does
 * not show (AGE-249), named in its title. Nothing when there are none.
 */
export default function WaitingTabs({ run, tabs, dot }: { run: RunInfo; tabs: RunTab[]; dot: string }) {
  const waiting = waitingElsewhere(run, tabs, dot);
  const label = waitingElsewhereLabel(waiting);
  if (!label) return null;
  return (
    <span className="tabs-waiting" title={`Waiting on you: ${waiting.map((t) => t.label).join(", ")}`}>
      {label}
    </span>
  );
}
