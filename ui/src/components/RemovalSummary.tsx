import { RemovalCopy } from "../lib/runRemoval";

/**
 * What a teardown removes and what survives it, as two labelled lists.
 *
 * The lists are the whole point of AGE-149: the old dialogs asked the user to
 * choose between "archive" and "delete" and then described the choice in one
 * sentence of prose, which is how "delete" came to read as catastrophic after a
 * merge and "archive" as expensive. Shown side by side, with the branch's real
 * state in them, the decision makes itself.
 *
 * Shared by the confirm dialogs so they describe one action rather than several
 * that sound different. The merge window's last step is not one of them: three
 * buttons there share one explanation, which `mergeTidyCopy` writes instead.
 */
export default function RemovalSummary({
  copy,
  checking,
  probeFailed,
}: {
  copy: RemovalCopy;
  /** The branch is still being read, so the lists are not final yet. */
  checking?: boolean;
  /** It could not be read at all. */
  probeFailed?: boolean;
}) {
  return (
    <div className="removal-body">
      <p>{copy.lead}</p>
      <div className="removal-lists">
        {copy.goes.length > 0 && (
          <div className="removal-col">
            <p className="removal-label">Removes</p>
            <ul className="removal-goes">
              {copy.goes.map((line) => (
                <li key={line}>{line}</li>
              ))}
            </ul>
          </div>
        )}
        {copy.stays.length > 0 && (
          <div className="removal-col">
            <p className="removal-label">Keeps</p>
            <ul className="removal-stays">
              {copy.stays.map((line) => (
                <li key={line}>{line}</li>
              ))}
            </ul>
          </div>
        )}
      </div>
      <BranchProbeNote checking={checking} probeFailed={probeFailed} />
      {copy.warning && <p className="removal-warn">{copy.warning}</p>}
    </div>
  );
}

/**
 * What the window says when it has no plan to show yet, or none at all. Shared
 * with the merge window so both hedge in the same words: a guess about a branch
 * reads as a promise about it.
 */
export function BranchProbeNote({
  checking,
  probeFailed,
}: {
  checking?: boolean;
  probeFailed?: boolean;
}) {
  if (checking) return <p className="removal-checking">Checking the branch…</p>;
  if (probeFailed) {
    return (
      <p className="removal-warn">
        Agency could not read this branch, so it cannot say here what happens to it. A branch that
        is the only copy of its work is kept either way.
      </p>
    );
  }
  return null;
}
