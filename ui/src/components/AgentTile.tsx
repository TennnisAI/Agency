import { useEffect, useState } from "react";
import { RunInfo, runPreview } from "../api";
import { useRuns } from "../store/runs";

function badgeClass(agent: string): string {
  if (agent === "claude") return "badge claude";
  if (agent === "pi") return "badge pi";
  if (agent === "hermes") return "badge hermes";
  return "badge";
}
function statusLabel(s: RunInfo["status"]): { cls: string; text: string } {
  if (s.state === "running") return { cls: "running", text: "running" };
  if (s.state === "exited") return { cls: "exited", text: `exited (${s.code})` };
  return { cls: "exited", text: "gone" };
}

export default function AgentTile({ run }: { run: RunInfo }) {
  const { setFocusedRun, setView } = useRuns();
  const [preview, setPreview] = useState("");

  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const p = await runPreview(run.id, 12);
        if (alive) setPreview(p);
      } catch {
        /* ignore */
      }
    };
    tick();
    const t = window.setInterval(tick, 1500);
    return () => { alive = false; window.clearInterval(t); };
  }, [run.id]);

  const st = statusLabel(run.status);
  return (
    <div className="tile" onClick={() => { setFocusedRun(run.id); setView("focus"); }}>
      <div className="tile-head">
        <span className={`dot ${st.cls}`} />
        <span className="tile-title">{run.prompt || run.branch}</span>
        <span className={badgeClass(run.agent)}>{run.agent}</span>
      </div>
      <div className="tile-meta">
        <code>{run.branch}</code>
        <span className="diffstat"><span className="add">+{run.added}</span> <span className="del">−{run.deleted}</span> · {run.files}f</span>
      </div>
      <pre className="tile-preview">{preview}</pre>
      <div className="tile-foot">{st.text}</div>
    </div>
  );
}
