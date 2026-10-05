import { useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";
import { listPopouts, requestReattach } from "../api";
import { PopoutEntry } from "../lib/popout";
import { IS_TAURI } from "../lib/platform";
import { unlistenQuietly } from "../lib/unlisten";

// The main window's copy of the popped-out list (AGE-252). One subscription for
// the whole window, however many panes ask: every agent pane, file tab and note
// tab checks it, and a listener and an IPC round trip apiece would be one per
// open tab.

let list: PopoutEntry[] = [];
const subscribers = new Set<() => void>();
let stop: (() => void) | null = null;

function refresh() {
  listPopouts()
    .then((next) => {
      list = next;
      subscribers.forEach((f) => f());
    })
    .catch(() => {});
}

function subscribe(onChange: () => void) {
  subscribers.add(onChange);
  if (!stop && IS_TAURI) {
    refresh();
    const sub = listen("popouts-changed", refresh);
    stop = () => { void sub.then(unlistenQuietly).catch(() => {}); };
  }
  return () => {
    subscribers.delete(onChange);
    if (subscribers.size === 0 && stop) {
      stop();
      stop = null;
    }
  };
}

export function usePopouts(): PopoutEntry[] {
  return useSyncExternalStore(subscribe, () => list);
}

/**
 * The main window's "Bring back". The popout hands the item back itself,
 * because it holds what has to come with it (a file's unsaved edits). One
 * whose page never loaded is brought back by the backend instead, so the
 * button cannot be a dead end (see `request_reattach` in popout.rs).
 */
export function bringBack(label: string) {
  requestReattach(label).catch(() => {});
}
