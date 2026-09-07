import { useEffect } from "react";
import { FileRoot, setOpenFile } from "../api";

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
 * The backend drops all of it while the user has sharing switched off.
 */
export function useReportOpenFile(root: FileRoot | null, path: string | null): void {
  const kind = root?.kind ?? null;
  const id = root?.id ?? null;
  useEffect(() => {
    const open = kind && id && path ? { root: { kind, id } as FileRoot, path } : null;
    // Nothing to say to the user if this fails: it is context an agent may ask
    // for, not an action they took.
    setOpenFile(open).catch(() => {});
  }, [kind, id, path]);
}
