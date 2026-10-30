import { useCallback, useEffect, useState } from "react";
import {
  CustomAgentInfo,
  CustomAgents,
  deleteCustomAgent,
  listCustomAgents,
  saveCustomAgent,
} from "../api";
import { agentLabel } from "../agents";
import { toastError } from "../lib/toast";
import ConfirmDialog from "./ConfirmDialog";
import FormDialog, { Field } from "./FormDialog";

type Draft = {
  // Set when editing: an edit keeps its id and stays in the scope it was found.
  id: string | null;
  scope: "user" | "project";
  name: string;
  description: string;
  instructions: string;
};

// The agents themselves, written once and handed to whichever harness runs them.
// A harness is an "Agent profile" in the section above this one; the two are
// separate because one says which CLI to launch and this says who it is.
export default function CustomAgentsSection({ projectId }: { projectId: string | null }) {
  const [data, setData] = useState<CustomAgents>({ agents: [], harnesses: [] });
  const [draft, setDraft] = useState<Draft | null>(null);
  const [error, setError] = useState("");
  const [deleting, setDeleting] = useState<CustomAgentInfo | null>(null);

  const refresh = useCallback(() => {
    listCustomAgents(projectId).then(setData).catch((e) => toastError(e, "Couldn't read custom agents"));
  }, [projectId]);
  useEffect(refresh, [refresh]);

  const openNew = () => {
    setError("");
    setDraft({ id: null, scope: "user", name: "", description: "", instructions: "" });
  };
  const openEdit = (a: CustomAgentInfo) => {
    setError("");
    setDraft({
      id: a.id,
      scope: a.scope,
      name: a.name,
      description: a.description,
      instructions: a.instructions,
    });
  };

  async function submit() {
    if (!draft) return;
    try {
      await saveCustomAgent(draft.scope, projectId, draft.id, draft.name, draft.description, draft.instructions);
      setDraft(null);
      refresh();
    } catch (e) {
      setError(String(e));
    }
  }

  return (
    <>
      <p className="settings-section-hint">
        An agent you write once and start in any of these harnesses:{" "}
        {data.harnesses.map(agentLabel).join(", ")}. Pick one from the add agent menu.
      </p>
      <div className="settings-card-list">
        {data.agents.map((a) => (
          <div key={a.id} className="settings-profile-card">
            <div className="settings-profile-head">
              <span className="settings-profile-name">{a.name}</span>
              <span className="settings-scope-tag" data-scope={a.scope === "project" ? "project" : "global"}>
                {a.scope === "project" ? "This project" : "Every project"}
              </span>
              <span className="spacer" />
              <button className="settings-ghost-btn" onClick={() => openEdit(a)}>Edit</button>
              <button className="settings-ghost-btn settings-del-btn" onClick={() => setDeleting(a)}>Delete</button>
            </div>
            {a.description && <div className="custom-agent-desc">{a.description}</div>}
          </div>
        ))}
        <button className="settings-add-profile" onClick={openNew}>+ New agent</button>
      </div>
      {draft && (
        <FormDialog
          title={draft.id ? `Edit ${draft.name || "agent"}` : "New agent"}
          submitLabel={draft.id ? "Save changes" : "Create agent"}
          submitDisabled={!draft.name.trim() || !draft.instructions.trim()}
          onSubmit={submit}
          onCancel={() => setDraft(null)}
        >
          {error && <div className="git-error">{error}</div>}
          <Field label="Name" hint="Shown in the add agent menu and on the run's tile.">
            <input
              className="settings-input"
              autoFocus
              placeholder="Reviewer"
              value={draft.name}
              onChange={(e) => setDraft({ ...draft, name: e.target.value })}
            />
          </Field>
          <Field label="Description" optional hint="One line on what it is for.">
            <input
              className="settings-input"
              placeholder="Reviews a diff for correctness"
              value={draft.description}
              onChange={(e) => setDraft({ ...draft, description: e.target.value })}
            />
          </Field>
          <Field label="Instructions" hint="Who this agent is and how it works. The harness gets these when it starts.">
            <textarea
              className="settings-input custom-agent-instructions"
              placeholder="You review diffs. Lead with what is wrong, and say why."
              value={draft.instructions}
              onChange={(e) => setDraft({ ...draft, instructions: e.target.value })}
            />
          </Field>
          {!draft.id && projectId && (
            <Field
              label="Available in"
              hint={<>This project keeps it in <code>.agency/agents/</code>, where you can commit it to share it.</>}
            >
              <select
                className="settings-input"
                value={draft.scope}
                onChange={(e) => setDraft({ ...draft, scope: e.target.value as Draft["scope"] })}
              >
                <option value="user">Every project</option>
                <option value="project">This project</option>
              </select>
            </Field>
          )}
        </FormDialog>
      )}
      {deleting && (
        <ConfirmDialog
          title={`Delete ${deleting.name}?`}
          body="Runs already started as this agent keep it. New runs can't pick it."
          confirmLabel="Delete"
          danger
          onConfirm={() => {
            const { scope, id } = deleting;
            setDeleting(null);
            deleteCustomAgent(scope, projectId, id).then(refresh).catch((e) => toastError(e, "Couldn't delete agent"));
          }}
          onCancel={() => setDeleting(null)}
        />
      )}
    </>
  );
}
