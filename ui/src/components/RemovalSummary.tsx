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
 * Shared by the confirm dialogs and by the merge window's last step so all
 * three describe one action rather than three that sound different.
 */
export default function RemovalSummary({
  copy,
  checking,
  probeFailed,
  hideLead,
}: {
  copy: RemovalCopy;
  /**
   * Drop the opening sentence. For a host that has already said what this is
   * about — the merge window, whose own heading is the lead — where repeating
   * it reads as the dialog talking to itself.
   */
  hideLead?: boolean;
  /** The branch is still being read, so the lists are not final yet. */
  checking?: boolean;
  /** It could not be read at all. */
  probeFailed?: boolean;
}) {
  return (
    <div className="removal-body">
      {!hideLead && <p>{copy.lead}</p>}
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
      {checking && <p className="removal-checking">Checking the branch…</p>}
      {probeFailed && (
        <p className="removal-warn">
          Agency could not read this branch, so it cannot say here what happens to it. A branch
          that is the only copy of its work is kept either way.
        </p>
      )}
      {copy.warning && <p className="removal-warn">{copy.warning}</p>}
    </div>
  );
}
