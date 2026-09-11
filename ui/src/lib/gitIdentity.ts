// Walking the user through "Author identity unknown" instead of showing it.
//
// A fresh git install has no user.name/user.email, so the first commit fails
// with "Author identity unknown / *** Please tell me who you are" (observed
// 2026-09-10 on Linux, right after git was installed from onboarding). The
// user cannot act on that text. This recognises it and raises a small
// name-and-email form; the backend sets the identity on the repository (local
// scope only, per the house rule that Agency never writes global git config).

/** True when a git error is really "no commit identity is configured". */
export function needsGitIdentity(err: unknown): boolean {
  const t = err instanceof Error ? err.message : String(err ?? "");
  return /Author identity unknown|Please tell me who you are|empty ident name|no name was given|no email was given/i.test(
    t,
  );
}

// A window event, not React context, so the setup dialog, the commit box and
// any api call site can raise the form the same way they toast. The host
// component (mounted at the shell root and under onboarding) listens.
export const GIT_IDENTITY_EVENT = "agency:git-identity";
// Fired once the identity is saved, so the surface that hit the error can
// retry the commit it was mid-way through.
export const GIT_IDENTITY_SET_EVENT = "agency:git-identity-set";
// Fired when the form is dismissed without saving. The surface that was
// waiting drops its pending retry and shows why the commit did not happen;
// without this the git panel sat with its error blanked and the retry armed,
// so a later save from another surface replayed a commit the user had
// abandoned.
export const GIT_IDENTITY_CANCELLED_EVENT = "agency:git-identity-cancelled";

export type GitIdentityOffer = { repoPath: string; reason: string };
export type GitIdentitySet = { repoPath: string };

/**
 * Raise the identity form for `repoPath`, with the failing error as its lede.
 * Returns false when no host is mounted, so the caller can fall back to a
 * toast.
 */
export function offerGitIdentity(repoPath: string, reason: string): boolean {
  if (typeof window === "undefined" || !window.__agencyGitIdentityHost) return false;
  window.dispatchEvent(new CustomEvent<GitIdentityOffer>(GIT_IDENTITY_EVENT, { detail: { repoPath, reason } }));
  return true;
}

declare global {
  interface Window {
    /** Set while a GitIdentityHost is mounted (see components/GitIdentityHost). */
    __agencyGitIdentityHost?: number;
  }
}
