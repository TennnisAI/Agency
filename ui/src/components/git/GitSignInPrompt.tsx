import { useEffect, useRef, useState } from "react";
import { GitAuthTarget, gitSignIn } from "../../api";
import { useModalKeys } from "../../hooks/useModalKeys";

// Asks for the username and password a remote turned the last action away
// for, as a quick input across the top of the window: a step for the username
// (skipped when git already named one), then one for the password or token.
// The credential is checked against origin before this closes, so a mistyped
// token is reported here rather than as the banner the user was clearing.
export default function GitSignInPrompt({
  taskId,
  target,
  username: knownUser,
  onSignedIn,
  onCancel,
}: {
  taskId: string;
  target: GitAuthTarget;
  // The username git named in its own prompt, which beats the remote URL's.
  username: string | null;
  onSignedIn: () => void;
  onCancel: () => void;
}) {
  const initialUser = knownUser ?? target.username ?? "";
  const [step, setStep] = useState<"username" | "password">(initialUser ? "password" : "username");
  const [username, setUsername] = useState(initialUser);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useModalKeys(onCancel, !busy);
  useEffect(() => { inputRef.current?.focus(); }, [step]);

  async function confirm() {
    if (busy) return;
    if (step === "username") {
      if (!username.trim()) return;
      setError("");
      setStep("password");
      return;
    }
    if (!password) return;
    setBusy(true);
    setError("");
    try {
      await gitSignIn(taskId, username.trim(), password);
      onSignedIn();
    } catch (e) {
      setError(String(e));
      setPassword("");
      setBusy(false);
    }
  }

  const host = target.base.replace(/^[a-z]+:\/\//i, "");
  const prompt = step === "username"
    ? `Username for '${target.base}'`
    : `Password for '${target.base.replace("://", `://${username.trim()}@`)}'`;

  return (
    <div className="palette-overlay" onMouseDown={busy ? undefined : onCancel}>
      <div className="palette git-signin" role="dialog" aria-modal="true" aria-label={`Sign in to ${host}`}
        onMouseDown={(e) => e.stopPropagation()}>
        <div className="git-signin-title">
          {`${host} turned down the saved credential. Sign in again to carry on.`}
        </div>
        <input
          key={step}
          ref={inputRef}
          className="palette-input"
          type={step === "password" ? "password" : "text"}
          autoComplete="off"
          spellCheck={false}
          disabled={busy}
          placeholder={step === "username" ? "Username" : "Password or personal access token"}
          value={step === "username" ? username : password}
          onChange={(e) => (step === "username" ? setUsername : setPassword)(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") { e.preventDefault(); void confirm(); }
            if (e.key === "Backspace" && step === "password" && !password && !busy) {
              setStep("username");
            }
          }}
        />
        {error && <div className="git-signin-error" role="alert">{error}</div>}
        <div className="palette-hint">
          {busy ? `Checking with ${host}…` : `${prompt}. Press Enter to confirm or Escape to cancel.`}
        </div>
      </div>
    </div>
  );
}
