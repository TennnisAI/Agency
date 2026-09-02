// Path actions shared by the file tree and the source-control changes list —
// the two places that offer a context menu over a worktree-relative path. They
// report their own outcome via toasts and never reject, so a menu item can call
// one directly.

import { FileRoot, absPath, addToGitignore, revealPath } from "../api";
import { baseName } from "./filePath";
import { toastError, toastInfo } from "./toast";

/** What the OS calls its file manager, for menu labels. */
export const revealLabel = navigator.platform.startsWith("Mac")
  ? "Reveal in Finder"
  : "Show in File Explorer";

export async function reveal(root: FileRoot, relPath: string): Promise<void> {
  try {
    await revealPath(root, relPath);
  } catch (e) {
    toastError(e, "Couldn't reveal");
  }
}

/** Copy the path's absolute location on disk. */
export async function copyAbsPath(root: FileRoot, relPath: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(await absPath(root, relPath));
    toastInfo("Path copied");
  } catch (e) {
    toastError(e, "Couldn't copy path");
  }
}

/** Copy the path as written in git — relative to the worktree root. */
export async function copyRelPath(relPath: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(relPath);
    toastInfo("Path copied");
  } catch (e) {
    toastError(e, "Couldn't copy path");
  }
}

/** Append to the root's .gitignore. Resolves true only if a new entry landed. */
export async function ignorePath(root: FileRoot, relPath: string): Promise<boolean> {
  try {
    const added = await addToGitignore(root, relPath);
    // A collapsed untracked folder arrives as git status prints it, with a
    // trailing slash, and baseName of ".npm-cache/" is the empty string — the
    // toast read "Added  to .gitignore" until the slash came off first.
    const name = baseName(relPath.replace(/\/+$/, ""));
    toastInfo(added ? `Added ${name} to .gitignore` : `${name} is already in .gitignore`);
    return added;
  } catch (e) {
    toastError(e, "Couldn't update .gitignore");
    return false;
  }
}
