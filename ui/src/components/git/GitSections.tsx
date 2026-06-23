import { useState } from "react";
import Resizer from "../Resizer";
import { usePaneWidth, loadFold, saveFold } from "../../hooks/usePaneWidth";

const HISTORY_MIN = 120;
const HISTORY_MAX = 800;
const FOLD_KEY = "git-history-folded";

export default function GitSections({
  changesPanel,
  historyPanel,
}: {
  changesPanel: React.ReactNode;
  historyPanel: React.ReactNode;
}) {
  const history = usePaneWidth("git-history-h", 220, HISTORY_MIN, HISTORY_MAX);
  const [folded, setFolded] = useState<boolean>(() =>
    typeof localStorage === "undefined" ? false : loadFold(localStorage, FOLD_KEY, false),
  );

  const toggleFold = () => {
    setFolded((f) => {
      const next = !f;
      try {
        if (typeof localStorage !== "undefined") saveFold(localStorage, FOLD_KEY, next);
      } catch {
        /* ignore quota / security errors */
      }
      return next;
    });
  };

  return (
    <div className="git-sections">
      <div className="git-section git-section-changes">
        <div className="git-section-head static">Changes</div>
        <div className="git-section-body">{changesPanel}</div>
      </div>

      {!folded && <Resizer width={history.width} min={HISTORY_MIN} max={HISTORY_MAX} onChange={history.setWidth} orientation="horizontal" />}

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
