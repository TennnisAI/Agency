import { useEffect, useMemo, useState } from "react";
import { createPrFromBranch, listProjectBranches } from "../../api";

// Pick a sensible default base branch: the conventional trunk if present,
// otherwise the first branch that isn't the chosen head. The head is always
// excluded — a PR's base and head must differ, and opening a PR while on `main`
// itself would otherwise default the base to `main` too.
function defaultBase(branches: string[], head: string): string {
  return (
    branches.find((b) => (b === "main" || b === "master") && b !== head) ??
    branches.find((b) => b !== head) ??
    ""
  );
}

// Open a PR from an existing branch, without going through an agent run. Lives
// in the Pull Requests tab's right pane while creating. `openPrBranches` are the
// head branches that already have an open PR — they're hidden from the picker so
// you can't try to open a duplicate.
export default function NewPrForm({
  projectId,
  openPrBranches,
  onCreated,
  onCancel,
}: {
  projectId: string;
  openPrBranches: string[];
  onCreated: (number: number) => void;
  onCancel: () => void;
}) {
  const [branches, setBranches] = useState<string[] | null>(null);
  const [head, setHead] = useState("");
  const [base, setBase] = useState("");
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  // Branches you can open a PR from: everything without an open PR already.
  const available = useMemo(
    () => (branches ?? []).filter((b) => !openPrBranches.includes(b)),
    [branches, openPrBranches],
  );

  useEffect(() => {
    listProjectBranches(projectId)
      .then((pb) => {
        setBranches(pb.branches);
        const avail = pb.branches.filter((b) => !openPrBranches.includes(b));
        const h = avail.includes(pb.current) ? pb.current : avail[0] ?? "";
        setHead(h);
        setBase(defaultBase(pb.branches, h));
      })
      .catch((e) => setError(String(e)));
    // openPrBranches is captured at open time; the form is transient.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projectId]);

  // Base can be any branch except the chosen head (a base with its own PR is
  // fine — it's the target, not the source).
  const baseChoices = useMemo(() => (branches ?? []).filter((b) => b !== head), [branches, head]);

  async function create() {
    if (!head || !base) return;
    setBusy(true);
    setError("");
    try {
      const pr = await createPrFromBranch(projectId, head, base, title.trim() || null, body.trim() || null);
      onCreated(pr.number);
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  if (branches === null && !error) {
    return <div className="pr-review-empty"><span className="spinner" /> Loading branches…</div>;
  }

  if (branches !== null && available.length === 0) {
    return (
      <div className="newpr">
        <div className="newpr-head">New pull request</div>
        <p className="pr-review-empty">Every branch already has an open pull request.</p>
        <div className="newpr-actions">
          <button className="git-iconbtn" onClick={onCancel}>Back</button>
        </div>
      </div>
    );
  }

  return (
    <div className="newpr">
      <div className="newpr-head">New pull request</div>
      {error && <div className="git-error">{error}</div>}
      <label className="newpr-field">
        <span>Branch</span>
        <select
          value={head}
          onChange={(e) => {
            const h = e.target.value;
            setHead(h);
            if (h === base) setBase(defaultBase(branches ?? [], h));
          }}
        >
          {available.map((b) => (
            <option key={b} value={b}>{b}</option>
          ))}
        </select>
      </label>
      <label className="newpr-field">
        <span>Base</span>
        <select value={base} onChange={(e) => setBase(e.target.value)}>
          {baseChoices.map((b) => (
            <option key={b} value={b}>{b}</option>
          ))}
        </select>
      </label>
      <label className="newpr-field">
        <span>Title</span>
        <input
          className="settings-input"
          placeholder={head || "Defaults to the branch name"}
          value={title}
          onChange={(e) => setTitle(e.target.value)}
        />
      </label>
      <label className="newpr-field newpr-field-body">
        <span>Description</span>
        <textarea
          className="settings-input"
          placeholder="Optional — a summary of the branch's commits is generated when left blank."
          value={body}
          onChange={(e) => setBody(e.target.value)}
        />
      </label>
      <div className="newpr-actions">
        <button className="git-iconbtn pr-approve" disabled={busy || !head || !base} onClick={create}>
          {busy ? "Creating…" : "Create pull request"}
        </button>
        <button className="git-iconbtn" disabled={busy} onClick={onCancel}>Cancel</button>
      </div>
    </div>
  );
}
