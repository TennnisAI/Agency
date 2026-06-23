import type { Extension } from "@codemirror/state";
import { javascript } from "@codemirror/lang-javascript";
import { json } from "@codemirror/lang-json";
import { rust } from "@codemirror/lang-rust";
import { css } from "@codemirror/lang-css";
import { html } from "@codemirror/lang-html";
import { markdown } from "@codemirror/lang-markdown";
import { python } from "@codemirror/lang-python";

/** Pick a CodeMirror language extension from a file path; [] for unknown types. */
export function languageExtension(path: string): Extension[] {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  switch (ext) {
    case "ts": case "tsx": return [javascript({ typescript: true, jsx: ext === "tsx" })];
    case "js": case "jsx": return [javascript({ jsx: ext === "jsx" })];
    case "json": return [json()];
    case "rs": return [rust()];
    case "css": return [css()];
    case "html": return [html()];
    case "md": case "markdown": return [markdown()];
    case "py": return [python()];
    default: return [];
  }
}
