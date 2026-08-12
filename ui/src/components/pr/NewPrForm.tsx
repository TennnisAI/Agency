import { useEffect, useMemo, useState } from "react";
import { ProjectBranches, createPrFromBranch, gitAutoFetch, listProjectBranches } from "../../api";

// Every branch a PR can involve: local ones first, then the ones that only
// exist on origin. Both are valid PR endpoints — GitHub only cares that the
// branch is on the remote, and a local branch gets pushed on create.
function allBranches(pb: ProjectBranches): string[] {
  return [...pb.branches, ...pb.remote];
}

// The conventional trunk, if the repo has one. This is the branch we merge
// *into*, so it's the default base regardless of what's currently checked out.
function trunk(branches: string[]): string | undefined {
  return branches.find((b) => b === "main" || b === "master");
}

// Pick a sensible default base branch: the conventional trunk if present,
// otherwise the first branch that isn't the chosen head. The head is always
// excluded — a PR's base and head must differ, and opening a PR while on the
// base itself would otherwise default the base to that same branch.
function defaultBase(branches: string[], head: string): string {
  const t = trunk(branches);
  if (t && t !== head) return t;
  return branches.find((b) => b !== head) ?? "";
}

// The head/base pair to preselect for a given set of branches.
function defaultPair(pb: ProjectBranches, openPrBranches: string[]): { head: string; base: string } {
  const all = allBranches(pb);
  const avail = all.filter((b) => !openPrBranches.includes(b));
  // Base defaults to the trunk (main/master) — the branch you merge into.
  const base = defaultBase(all, "");
  // Head defaults to the branch you're on (the agent/feature branch), as long
  // as it isn't the base and can still open a PR; otherwise the first available
  // branch that isn't the base.
  const head =
    avail.includes(pb.current) && pb.current !== base
      ? pb.current
      : avail.find((x) => x !== base) ?? avail[0] ?? "";
  // If the trunk happened to be the only available head, fall back so base and
  // head still differ.
  return { head, base: base !== head ? base : defaultBase(all, head) };
}

// The <option>s for a branch picker. Remote-only branches are grouped under
// origin so it's clear which ones aren't checked out anywhere locally; with no
// remote-only branches there's nothing to distinguish, so the groups are
// dropped rather than shown with one empty half.
function BranchOptions({ local, remote }: { local: string[]; remote: string[] }) {
  const options = (bs: string[]) => bs.map((b) => <option key={b} value={b}>{b}</option>);
  if (remote.length === 0) return <>{options(local)}</>;
  return (
    <>
      {local.length > 0 && <optgroup label="Local">{options(local)}</optgroup>}
      <optgroup label="origin">{options(remote)}</optgroup>
    </>
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
  const [pb, setPb] = useState<ProjectBranches | null>(null);
  const [head, setHead] = useState("");
  const [base, setBase] = useState("");
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  // Branches you can open a PR from: everything without an open PR already.
  const available = useMemo(
    () => (pb ? allBranches(pb).filter((b) => !openPrBranches.includes(b)) : []),
    [pb, openPrBranches],
  );

  useEffect(() => {
    let stopped = false;
    listProjectBranches(projectId)
      .then((next) => {
        if (stopped) return;
        setPb(next);
        const pair = defaultPair(next, openPrBranches);
        setHead(pair.head);
        setBase(pair.base);
        // The remote-only branches above come from remote-tracking refs, which
        // are only as fresh as the last fetch — a branch pushed from another
        // machine a minute ago wouldn't be in the list. Fetch (the backend
        // throttles per project) and re-read if anything actually came in. Done
        // after the first read so the form appears immediately, and silently:
        // an offline remote just means the local view stands.
        return gitAutoFetch(`project:${projectId}`)
          .then((fetched) => (fetched && !stopped ? listProjectBranches(projectId) : null))
          .then((fresher) => {
            if (!fresher || stopped) return;
            setPb(fresher);
            // Keep whatever is selected; only reset when a chosen branch is
            // gone (deleted upstream and pruned), since a select showing a
            // branch that no longer exists would create against nothing.
            const all = allBranches(fresher);
            const fresh = defaultPair(fresher, openPrBranches);
            setHead((h) => (all.includes(h) ? h : fresh.head));
            setBase((b) => (all.includes(b) ? b : fresh.base));
          })
          .catch(() => {});
      })
      .catch((e) => !stopped && setError(String(e)));
    return () => {
      stopped = true;
    };
    // openPrBranches is captured at open time; the form is transient.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projectId]);

  // Base can be any branch except the chosen head (a base with its own PR is
  // fine — it's the target, not the source).
  const baseChoices = useMemo(
    () => (pb ? allBranches(pb).filter((b) => b !== head) : []),
    [pb, head],
  );
  const split = (bs: string[]) => ({
    local: bs.filter((b) => !pb?.remote.includes(b)),
    remote: bs.filter((b) => pb?.remote.includes(b)),
  });

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

  if (pb === null && !error) {
    return <div className="pr-review-empty"><span className="spinner" /> Loading branches…</div>;
  }

  if (pb !== null && available.length === 0) {
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
            if (h === base) setBase(defaultBase(pb ? allBranches(pb) : [], h));
          }}
        >
          <BranchOptions {...split(available)} />
        </select>
      </label>
      <label className="newpr-field">
        <span>Base</span>
        <select value={base} onChange={(e) => setBase(e.target.value)}>
          <BranchOptions {...split(baseChoices)} />
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
          placeholder="Optional. A summary of the branch's commits is generated when left blank."
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
