// Recognising a push, pull or fetch that failed for want of a credential.
//
// An expired token made Sync fail with "fatal: could not read Password for
// 'https://…': terminal prompts disabled" in the error banner, which says what
// is wrong and offers no way to fix it. This recognises that failure so the
// panel can ask for the new password instead, the way an editor's git
// integration does.

/** True when git was turned away for want of a working username and password. */
export function needsGitCredentials(err: unknown): boolean {
  const t = err instanceof Error ? err.message : String(err ?? "");
  return /could not read (username|password)|terminal prompts disabled|authentication failed|invalid username or password/i.test(
    t,
  );
}

/** The username git named in its prompt ("Password for 'https://nic@host'"), if any. */
export function promptedUsername(err: unknown): string | null {
  const t = err instanceof Error ? err.message : String(err ?? "");
  const m = /for '[a-z]+:\/\/([^@'/:]+)(?::[^@'/]*)?@/i.exec(t);
  return m ? decodeURIComponent(m[1]) : null;
}
