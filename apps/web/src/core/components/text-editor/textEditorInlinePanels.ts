import { EditorState } from "@codemirror/state";
import { Decoration, EditorView, WidgetType } from "@codemirror/view";

export interface TextEditorPanelMount { readonly at: number; readonly element: HTMLElement }

class InlinePanel extends WidgetType {
  constructor(readonly element: HTMLElement) { super(); }
  override eq(other: InlinePanel): boolean { return this.element === other.element; }
  override toDOM(): HTMLElement { return this.element; }
  override ignoreEvent(): boolean { return true; }
}

/** Content panels participate in the document flow, never in the overlay stack. */
export function textEditorInlinePanels(state: EditorState, panels: readonly TextEditorPanelMount[]) {
  return EditorView.decorations.of(Decoration.set(panels.map(panel => Decoration.widget({
    widget: new InlinePanel(panel.element), block: true, side: 1,
  }).range(state.doc.lineAt(Math.min(state.doc.length, Math.max(0, panel.at))).to)), true));
}
