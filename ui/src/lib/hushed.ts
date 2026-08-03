import { useEffect, useState } from "react";

// Messages you can silence. Some explanations earn their keep the first few
// times and turn into wallpaper once the workflow is habit, so each one here
// can be switched off from where it appears and switched back on in Settings.
// A per-machine preference (localStorage): what you already understand is not
// a property of the project.

export type HushId = "merge-cleanup" | "merge-delete";

export type Hushable = {
  id: HushId;
  /** How the message is named in Settings. */
  label: string;
  /** Where it shows up, so a row in Settings is recognisable. */
  hint: string;
};

// Only messages that are safe to lose belong here: an explanation of two
// reversible choices, and a confirm for work that is already merged. Confirms
// for real, unrecoverable loss (discarding an unmerged agent) stay put.
export const HUSHABLE: Hushable[] = [
  {
    id: "merge-cleanup",
    label: "What archiving and deleting an agent do",
    hint: "The note under a finished merge.",
  },
  {
    id: "merge-delete",
    label: "Confirm before deleting a merged agent",
    hint: "Deletes straight away instead of asking. Only when the merge landed cleanly.",
  },
];

const KEY_PREFIX = "agency:hushed:";

/** Fired on every change so open views re-read their own message. */
export const HUSHED_EVENT = "agency:hushed-changed";

export function isHushed(id: HushId, storage: Pick<Storage, "getItem"> = localStorage): boolean {
  try {
    return storage.getItem(KEY_PREFIX + id) === "1";
  } catch {
    return false;
  }
}

export function setHushed(
  id: HushId,
  hushed: boolean,
  storage: Pick<Storage, "setItem" | "removeItem"> = localStorage,
): void {
  try {
    if (hushed) storage.setItem(KEY_PREFIX + id, "1");
    else storage.removeItem(KEY_PREFIX + id);
  } catch {
    /* storage unavailable */
  }
  // Guarded for the test environment, which has storage stubs but no window.
  if (typeof window !== "undefined") window.dispatchEvent(new CustomEvent(HUSHED_EVENT));
}

/**
 * Live read of one message's setting. Subscribed to the change event so a
 * Settings toggle reaches a modal that is already open behind it.
 */
export function useHushed(id: HushId): [boolean, (hushed: boolean) => void] {
  const [hushed, set] = useState(() => isHushed(id));
  useEffect(() => {
    const reread = () => set(isHushed(id));
    window.addEventListener(HUSHED_EVENT, reread);
    return () => window.removeEventListener(HUSHED_EVENT, reread);
  }, [id]);
  return [hushed, (next: boolean) => setHushed(id, next)];
}
