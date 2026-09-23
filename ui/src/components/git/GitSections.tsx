import { useState } from "react";
import Resizer from "../Resizer";
import { usePaneWidth, loadFold, saveFold } from "../../hooks/usePaneWidth";

const HISTORY_MIN = 120;
const HISTORY_MAX = 800;
const FOLD_KEY = "git-history-folded";
const CHECKPOINTS_FOLD_KEY = "git-checkpoints-folded";

function loadFolded(key: string): boolean {
  return typeof localStorage === "undefined" ? false : loadFold(localStorage, key, false);
}

function saveFolded(key: string, folded: boolean) {
  try {
    if (typeof localStorage !== "undefined") saveFold(localStorage, key, folded);
  } catch {
    /* ignore quota / security errors */
  }
}

export default function GitSections({
  changesPanel,
  checkpointsPanel,
  historyPanel,
}: {
  changesPanel: React.ReactNode;
  /** A run's workspace checkpoints; absent for the project checkout itself. */
  checkpointsPanel?: React.ReactNode;
  historyPanel: React.ReactNode;
}) {
  const history = usePaneWidth("git-history-h", 220, HISTORY_MIN, HISTORY_MAX);
  const [folded, setFolded] = useState<boolean>(() => loadFolded(FOLD_KEY));
  const [cpFolded, setCpFolded] = useState<boolean>(() => loadFolded(CHECKPOINTS_FOLD_KEY));

  const toggleFold = () => {
    setFolded((f) => {
      saveFolded(FOLD_KEY, !f);
      return !f;
    });
  };
  const toggleCheckpoints = () => {
    setCpFolded((f) => {
      saveFolded(CHECKPOINTS_FOLD_KEY, !f);
      return !f;
    });
  };

  return (
    <div className="git-sections">
      <div className="git-section git-section-changes">
        <div className="git-section-head static">Changes</div>
        <div className="git-section-body">{changesPanel}</div>
      </div>

      {checkpointsPanel && (
        <div className={`git-section git-section-checkpoints ${cpFolded ? "folded" : ""}`}>
          <button className="git-section-head" onClick={toggleCheckpoints} aria-expanded={!cpFolded}>
            <span className="chev">{cpFolded ? "▸" : "▾"}</span> Checkpoints
          </button>
          {!cpFolded && <div className="git-section-body">{checkpointsPanel}</div>}
        </div>
      )}

      {!folded && <Resizer size={history.width} min={HISTORY_MIN} max={HISTORY_MAX} onChange={history.setWidth} orientation="horizontal" side="right" />}

      <div
        className={`git-section git-section-history ${folded ? "folded" : ""}`}
        style={folded ? undefined : { height: history.width, flexShrink: 0 }}
      >
        <button className="git-section-head" onClick={toggleFold} aria-expanded={!folded}>
          <span className="chev">{folded ? "▸" : "▾"}</span> History
        </button>
        {!folded && <div className="git-section-body">{historyPanel}</div>}
      </div>
    </div>
  );
}
