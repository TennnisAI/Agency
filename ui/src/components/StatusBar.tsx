import { useEffect, useState } from "react";
import { runBranches } from "../api";

export default function StatusBar({
  projectName,
  focusedRunId,
}: {
  projectName: string | null;
  focusedRunId: string | null;
}) {
  const [branches, setBranches] = useState<{ branch: string; base: string } | null>(null);

  useEffect(() => {
    if (!focusedRunId) { setBranches(null); return; }
    let alive = true;
    const load = () => runBranches(focusedRunId)
      .then((b) => { if (alive) setBranches(b); })
      .catch(() => { if (alive) setBranches(null); });
    load();
    const id = setInterval(load, 2000);
    return () => { alive = false; clearInterval(id); };
  }, [focusedRunId]);

  const { branch, base } = branches ?? { branch: "", base: "" };
  return (
    <footer className="statusbar">
      <span className="statusbar-left">
        <span>{projectName ?? "no project"}</span>
        {branch && (
          <span className="statusbar-branch" title="source branch → merge target">
            ⎇ {branch}{base && base !== branch ? ` → ${base}` : ""}
          </span>
        )}
      </span>
      <span className="kbd-hints">⌘N new · ⌘G source · ⌘↵ approve</span>
    </footer>
  );
}
