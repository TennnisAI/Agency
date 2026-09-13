import { useEffect, useRef, useState } from "react";
import { getGitIdentity, setGitIdentity } from "../api";
import { useModalKeys } from "../hooks/useModalKeys";
import ModalBackdrop from "./ModalBackdrop";

// The name and email git signs commits with. Set on the repository, not the
// user's global git config (the house rule); a worktree shares its
// repository's config, so this reaches every worktree Agency cuts from it.
export default function GitIdentityDialog({
  repoPath,
  reason,
  onSaved,
  onClose,
}: {
  repoPath: string;
  reason: string;
  onSaved: () => void;
  onClose: () => void;
}) {
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const nameRef = useRef<HTMLInputElement>(null);

  useModalKeys(onClose, !busy);

  // Prefill from whatever git already has (a half-set identity, or a global
  // one that some other tool wrote), so the common case is one click.
  useEffect(() => {
    getGitIdentity(repoPath)
      .then((id) => {
        if (id.name) setName(id.name);
        if (id.email) setEmail(id.email);
      })
      .catch(() => {});
  }, [repoPath]);

  useEffect(() => {
    nameRef.current?.focus();
  }, []);

  const emailOk = /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.trim());
  const ready = name.trim().length > 0 && emailOk && !busy;

  async function save() {
    if (!ready) return;
    setBusy(true);
    setError("");
    try {
      await setGitIdentity(repoPath, name.trim(), email.trim());
      onSaved();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  return (
    <ModalBackdrop onBackdropClick={busy ? undefined : onClose}>
      <div
        className="modal confirm"
        role="dialog"
        aria-modal="true"
        aria-label="Set your git identity"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h3>Tell git who you are</h3>
          <button className="modal-x" onClick={onClose}>✕</button>
        </div>
        <div className="modal-body">
          <p className="modal-note">
            Git stamps every commit with a name and email, and this machine has none set yet.
            Agency saves them for this repository.
          </p>
          <label className="settings-form-label" htmlFor="git-id-name">Name</label>
          <input
            id="git-id-name"
            ref={nameRef}
            className="settings-input"
            placeholder="Ada Lovelace"
            value={name}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") void save(); }}
          />
          <label className="settings-form-label" htmlFor="git-id-email">Email</label>
          <input
            id="git-id-email"
            className="settings-input"
            type="email"
            placeholder="ada@example.com"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter") void save(); }}
          />
          {reason && <div className="git-identity-reason">{reason}</div>}
          {error && <div className="git-error">{error}</div>}
        </div>
        <div className="modal-foot">
          <button className="btn-secondary" onClick={onClose}>Cancel</button>
          <button className="btn-primary" disabled={!ready} onClick={() => void save()}>
            {busy ? "Saving…" : "Save and continue"}
          </button>
        </div>
      </div>
    </ModalBackdrop>
  );
}
