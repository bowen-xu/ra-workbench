import type {EditorView} from "@tiptap/pm/view";
import {NodeSelection, Selection} from "@tiptap/pm/state";
import {textBlockLocation} from "./blockTextEditing";

export interface MediaBlockEditor {
  readonly prosemirrorView: EditorView;
  readonly isEditable: boolean;
}

/** Return an editing control's focus to the actual resource content node. */
export function focusMediaBlock(editor: MediaBlockEditor, blockId: string): boolean {
  if (!editor.isEditable) return false;
  const view = editor.prosemirrorView, block = textBlockLocation(view.state.doc, blockId);
  if (!block) return false;
  const selection = block.node.firstChild?.type.name === "table"
      ? Selection.near(view.state.doc.resolve(block.pos + block.node.firstChild.nodeSize), -1)
      : NodeSelection.create(view.state.doc, block.pos + 1);
  view.dispatch(view.state.tr.setSelection(selection).scrollIntoView());
  view.focus();
  return true;
}
