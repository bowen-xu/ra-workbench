import { ToolbarItem } from "@ariakit/react";
import type { BlockNoteEditor } from "@blocknote/core";
import { useBlockNoteEditor, useEditorState } from "@blocknote/react";
import { closeHistory } from "@tiptap/pm/history";
import { TextSelection } from "@tiptap/pm/state";
import { Radical } from "lucide-react";
import { useEffect } from "react";
import { ControlHint } from "../ControlHint";

type FormulaEditor = Pick<BlockNoteEditor, "isEditable" | "prosemirrorState" | "pmSchema" | "transact" | "focus">;

function selectedFormulaSource(editor: FormulaEditor): string | undefined {
  if (!editor.isEditable || !editor.pmSchema.nodes.inlineFormula) return undefined;
  const { doc, selection } = editor.prosemirrorState;
  const { from, to, $from, $to } = selection;
  if (!(selection instanceof TextSelection) || selection.empty || !$from.sameParent($to)
    || $from.parent.type.spec.code || $from.parent.type.name === "codeBlock") return undefined;
  let textOnly = true;
  doc.nodesBetween(from, to, node => {
    if ((node.isInline && !node.isText && node.type.name !== "hardBreak")
      || node.marks.some(mark => mark.type.name === "code")) textOnly = false;
  });
  const source = doc.textBetween(from, to, "", "\n");
  return textOnly && source.trim() && source.length <= 32_768 ? source : undefined;
}

/** Replace only a plain text range; atom identity and block structure stay intact. */
function markSelectionAsFormula(editor: FormulaEditor): boolean {
  const source = selectedFormulaSource(editor);
  const formula = editor.pmSchema.nodes.inlineFormula;
  if (source === undefined || !formula) return false;
  editor.transact(transaction => {
    closeHistory(transaction);
    transaction.replaceSelectionWith(formula.create({ source }), false).setStoredMarks([]);
  });
  editor.focus();
  return true;
}

export function MarkSelectionAsFormulaButton(): React.JSX.Element | null {
  const editor = useBlockNoteEditor();
  const enabled = useEditorState({ editor, selector: ({ editor }) => selectedFormulaSource(editor) !== undefined });
  if (!editor.pmSchema.nodes.inlineFormula) return null;
  return <ControlHint content="标记为公式" shortcut="⌘⇧E"><ToolbarItem
    aria-label="标记为公式" aria-keyshortcuts="Meta+Shift+E Control+Shift+E" disabled={!enabled}
    className="bn-ak-button bn-ak-secondary bn-button" onMouseDown={event => event.preventDefault()}
    onClick={() => markSelectionAsFormula(editor)}><Radical aria-hidden="true"><path d="m16 10 5 7m0-7-5 7" /></Radical></ToolbarItem></ControlHint>;
}

/** Mounted with the editor, so the shortcut also works while the toolbar is hidden. */
export function FormulaSelectionShortcut(): null {
  const editor = useBlockNoteEditor();
  const root = useEditorState({ selector: context => context.editor?.domElement });
  useEffect(() => {
    const host = root?.closest(".block-editor");
    if (!root || !host) return;
    const keyDown = (event: Event) => {
      if (!(event instanceof KeyboardEvent) || event.key.toLowerCase() !== "e" || !event.shiftKey
        || !(event.metaKey || event.ctrlKey) || event.altKey || event.isComposing || event.keyCode === 229
        || editor.prosemirrorView.composing || !(event.target instanceof Element) || !root.contains(event.target)
        || event.target.closest('input, textarea, button, select, [contenteditable="false"]')) return;
      if (markSelectionAsFormula(editor)) { event.preventDefault(); event.stopImmediatePropagation(); }
    };
    host.addEventListener("keydown", keyDown, true);
    return () => host.removeEventListener("keydown", keyDown, true);
  }, [editor, root]);
  return null;
}
