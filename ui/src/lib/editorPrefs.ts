// Persisted file-editor preferences. Word wrap defaults ON; toggling broadcasts
// a `wordwrapchange` event so open editors reconfigure live without a remount.
export const WORD_WRAP_KEY = "fileWordWrap";

export function getWordWrap(storage: Pick<Storage, "getItem"> = localStorage): boolean {
  try {
    const raw = storage.getItem(WORD_WRAP_KEY);
    return raw === null ? true : raw === "1";
  } catch {
    return true;
  }
}

export function setWordWrap(on: boolean): void {
  try {
    localStorage.setItem(WORD_WRAP_KEY, on ? "1" : "0");
  } catch {
    /* storage unavailable */
  }
  window.dispatchEvent(new CustomEvent("wordwrapchange", { detail: on }));
}
