import { useEffect } from "react";
import { FileRoot, setOpenFile } from "../api";

/** The last focus reported, for `resendOpenFile` to send again. */
let reported: { root: FileRoot; path: string } | null = null;

/**
 * Report which file this view has in focus, for the `editor_open_file` MCP
 * tool (AGE-200). One cell app-wide: whichever view reported last is what an
 * agent hears, which is the same order the user moved through them in.
 *
 * Deliberately not cleared when the view unmounts. Switching to the Agents tab
 * puts the file behind another tab; it does not close it, and "the user has no
 * file open" is the wrong answer to give an agent they walked over to the
 * moment after reading a note. A file that really is closed reports as `null`
 * through `path` instead, and an app restart starts empty.
 *
 * Sends on every change including the first, and sends `null` for "nothing
 * open", so the state the backend holds is only ever what the last view said.
 * The backend drops all of it while the user has sharing switched off, which is
 * what `resendOpenFile` is for.
 */
export function useReportOpenFile(root: FileRoot | null, path: string | null): void {
  const kind = root?.kind ?? null;
  const id = root?.id ?? null;
  useEffect(() => {
    reported = kind && id && path ? { root: { kind, id } as FileRoot, path } : null;
    // Nothing to say to the user if this fails: it is context an agent may ask
    // for, not an action they took.
    setOpenFile(reported).catch(() => {});
  }, [kind, id, path]);
}

/**
 * Send the current focus again, after the user turns sharing on. Every report
 * made while the switch was off was dropped by the backend, so the cell it
 * answers from is empty: without this, a user who ticked the setting with a
 * file in front of them was told by `editor_open_file` that "sharing is on, and
 * there is simply no file in focus" until they next switched file or tab.
 *
 * Call it only once the setting has been saved, or the backend drops this one
 * too.
 */
export function resendOpenFile(): void {
  setOpenFile(reported).catch(() => {});
}
