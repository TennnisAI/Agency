import { CloneProgress } from "../api";

/**
 * The phase / percent / detail readout every long backend job reports through:
 * clone, push, spawning a worktree, tearing one down. One component so a job
 * that gains progress gets the same shape as the rest for free, instead of a
 * fourth hand-rolled copy of the same three divs.
 *
 * `fallback` is the phase to show before the first update lands, so the bar is
 * never blank in the window between the click and the backend's first step.
 * A `null` percent sweeps an indeterminate bar: honest about jobs git reports
 * no fraction for (removing a worktree, killing a session).
 */
export default function ProgressReadout({
  progress,
  fallback,
}: {
  progress: CloneProgress | null;
  fallback: string;
}) {
  return (
    <div className="clone-progress" role="status" aria-live="polite">
      <div className="clone-progress-head">
        <span className="clone-progress-phase">{progress ? progress.phase : fallback}</span>
        {progress?.percent != null && (
          <span className="clone-progress-pct">{progress.percent}%</span>
        )}
      </div>
      <div className="clone-progress-track">
        <div
          className={`clone-progress-bar${progress?.percent == null ? " indeterminate" : ""}`}
          style={progress?.percent != null ? { width: `${progress.percent}%` } : undefined}
        />
      </div>
      {progress?.detail && <div className="clone-progress-detail">{progress.detail}</div>}
    </div>
  );
}
