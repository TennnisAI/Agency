// Turning "this failed because git is not installed" into an offer to
// install it.
//
// Observed 2026-09-10 on Linux: starting an agent in a folder toasted
// "Couldn't start claude: could not run git in /home/nic/Documents/Project1
// to tell whether it is a repository; check that git is installed and the
// folder is available". True, and useless: the user had no terminal to
// install git from and no hint that the app could. The backend now says
// "git is not installed" outright where it can tell (setup.rs
// `git_spawn_error`, state.rs `require_gitless_known`); this recognises that
// sentence and raises the tool dialog instead of the toast.

import { toastError } from "./toast";

export type ToolId = "git" | "node" | "gh";

/** The tool an error is about, when it says a tool is missing. */
export function missingToolFor(error: unknown): ToolId | null {
  const text = error instanceof Error ? error.message : String(error ?? "");
  if (/\bgit is not installed\b/i.test(text)) return "git";
  if (/\bgh is not installed\b/i.test(text)) return "gh";
  return null;
}

// A window event, not React context, so the run store and the API call sites
// (which have no component) can raise the dialog the same way they toast.
export const MISSING_TOOL_EVENT = "agency:missing-tool";

export type MissingToolDetail = { tool: ToolId; reason: string };

/**
 * Raise the tool dialog for `tool`, with the error that led here as its lede.
 * Returns false when no listener is mounted, so the caller can toast instead.
 */
export function offerToolInstall(tool: ToolId, reason: string): boolean {
  if (typeof window === "undefined" || !window.__agencyToolHost) return false;
  window.dispatchEvent(new CustomEvent<MissingToolDetail>(MISSING_TOOL_EVENT, { detail: { tool, reason } }));
  return true;
}

declare global {
  interface Window {
    /** Set while a ToolInstallHost is mounted (see components/ToolInstallHost). */
    __agencyToolHost?: number;
  }
}

/**
 * Report a failed action: the tool dialog when the error names a missing
 * tool and a host is mounted, the usual error toast otherwise. Same signature
 * as `toastError`, so a call site swaps one for the other.
 */
export function reportFailure(err: unknown, context: string): void {
  const tool = missingToolFor(err);
  const raw = err instanceof Error ? err.message : String(err);
  if (tool && offerToolInstall(tool, `${context}: ${raw}`)) return;
  toastError(err, context);
}
