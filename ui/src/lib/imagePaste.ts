// Pasting an image into a markdown editor: the editor that owns the note saves
// the file and links it at the selection.
//
// The handler sits in a facet rather than in each editor's own paste handler
// because a table cell is edited by a small editor of its own (mdTable.ts), and
// the note's editor never sees that editor's events. Before the cell could
// reach the handler here, an image pasted into a table cell did nothing at all.

import { Extension, Facet } from "@codemirror/state";
import { EditorView } from "@codemirror/view";

/** Saves the pasted images and links them at `view`'s selection. False when
 *  this editor takes no files, so the paste goes ahead as text. */
export type PasteImages = (view: EditorView, files: File[]) => boolean;

const pasteImagesFacet = Facet.define<PasteImages, PasteImages | null>({
  combine: (values) => values[0] ?? null,
});

/** Hand a paste carrying images to the handler `view` was given. */
export function pasteImages(e: ClipboardEvent, view: EditorView): boolean {
  const handler = view.state.facet(pasteImagesFacet);
  const files = [...(e.clipboardData?.files ?? [])].filter((f) => f.type.startsWith("image/"));
  if (!handler || files.length === 0 || !handler(view, files)) return false;
  e.preventDefault();
  return true;
}

/** Image paste for an editor, and for the table cells edited inside it. */
export function imagePaste(handler: PasteImages): Extension {
  return [
    pasteImagesFacet.of(handler),
    EditorView.domEventHandlers({ paste: (e, view) => pasteImages(e, view) }),
  ];
}
