import { useEffect } from "react";
import type { Slice } from "@tiptap/pm/model";
import type { NoteEditorInstance } from "./noteEditorSchema";
import { noteClipboardText } from "./noteClipboardText";

/** Runs in the wrapper's bubble phase, after BlockNote has written its rich MIME
 * representations. Standard HTML also carries the complete block structure so
 * platforms that drop the custom MIME type can still preserve empty blocks. */
export function NoteClipboard({ editor }: { readonly editor: NoteEditorInstance }): null {
  useEffect(() => {
    let detach: (() => void) | undefined;
    const attach = () => {
      detach?.();
      const workspace = editor.domElement?.closest(".block-editor");
      if (!workspace) return;
      let captured: { readonly event: Event; readonly slice: Slice } | undefined;
      const capture = (event: Event) => {
        const selection = editor.prosemirrorView.state.selection;
        captured = selection.empty ? undefined : { event, slice: selection.content() };
      };
      const copy = (event: Event) => {
        const clipboard = event as ClipboardEvent;
        const data = clipboard.clipboardData;
        if (!data) return;
        const view = editor.prosemirrorView;
        // Cut has already deleted the engine selection by the bubble phase.
        // Snapshot the same selection, rather than maintaining separate block IDs.
        const slice = captured?.event === event ? captured.slice : undefined;
        captured = undefined;
        if (clipboard.defaultPrevented && slice) {
          const text = noteClipboardText(slice);
          if (text !== undefined) {
            data.setData("text/plain", text);
            data.setData("text/html", data.getData("blocknote/html") || view.serializeForClipboard(slice).dom.innerHTML);
          }
        }
      };
      for (const type of ["copy", "cut"]) {
        workspace.addEventListener(type, capture, true);
        workspace.addEventListener(type, copy);
      }
      detach = () => {
        for (const type of ["copy", "cut"]) {
          workspace.removeEventListener(type, capture, true);
          workspace.removeEventListener(type, copy);
        }
      };
    };
    const stopMount = editor.onMount(attach);
    attach();
    return () => { stopMount(); detach?.(); };
  }, [editor]);
  return null;
}
