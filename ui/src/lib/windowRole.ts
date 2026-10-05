// Which window this page is: the main one, or an item popped out into a window
// of its own (AGE-252). Both load the same bundle; main.tsx picks the root.
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { IS_TAURI } from "./platform";
import { isPopoutLabel } from "./popout";

export const WINDOW_LABEL: string = IS_TAURI ? getCurrentWebviewWindow().label : "main";

export const IN_POPOUT: boolean = isPopoutLabel(WINDOW_LABEL);
