import { closeHistory } from "@tiptap/pm/history";
import type { BlockNoteEditor, StyleSchema } from "@blocknote/core";

type FormattingEditor = Pick<BlockNoteEditor, "transact" | "pmSchema" | "focus"> & {
  readonly schema: { readonly styleSchema: StyleSchema };
};

/** Clear schema marks in one history event without changing links or nodes. */
export function clearBlockEditorFormatting(editor: FormattingEditor): void {
  // removeStyles issues one Tiptap command per mark. Use a single native mark
  // transaction so undo restores every style, including mixed selections.
  editor.transact(transaction => {
    closeHistory(transaction);
    for (const name of Object.keys(editor.schema.styleSchema)) {
      const mark = editor.pmSchema.marks[name];
      if (!mark) continue;
      transaction.removeMark(transaction.selection.from, transaction.selection.to, mark);
      transaction.removeStoredMark(mark);
    }
  });
  editor.focus();
}
