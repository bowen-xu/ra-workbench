import { EditorState, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";
import { getChunks } from "@codemirror/merge";
import { createEditorDiffChunkActions } from "./editorDiffChunkActions";
type ResolveChunk = (view: EditorView, position: number, action: "keep" | "undo") => void;
class ChunkActions extends WidgetType {
    constructor(readonly position: number, readonly number: number, readonly resolve: ResolveChunk, readonly disabled: boolean, readonly labels?: { readonly undo?: string; readonly keep?: string }) { super(); }
    override eq(other: ChunkActions): boolean { return this.position === other.position && this.number === other.number && this.disabled === other.disabled && this.labels?.undo === other.labels?.undo && this.labels?.keep === other.labels?.keep; }
    override toDOM(view: EditorView): HTMLElement {
        return createEditorDiffChunkActions({ document: view.dom.ownerDocument, number: this.number, disabled: this.disabled,
            onResolve: action => this.resolve(view, this.position, action), ...(this.labels ? { labels: this.labels } : {}) });
    }
    override ignoreEvent(): boolean { return true; }
}
export function chunkActionDecorations(resolve: ResolveChunk, disabled = false, incoming: readonly { readonly from: number; readonly to: number }[] = [], labels?: { readonly undo?: string; readonly keep?: string }): StateField<DecorationSet> {
    const build = (state: EditorState): DecorationSet => Decoration.set((getChunks(state)?.chunks ?? []).flatMap((chunk, index) => incoming.some(group => group.from === chunk.fromB && group.to === Math.min(state.doc.length, chunk.toB)) ? [] : [Decoration.widget({
        widget: new ChunkActions(chunk.fromB, index + 1, resolve, disabled, labels), block: true, side: 1,
    }).range(chunk.endB)]), true);
    return StateField.define<DecorationSet>({
        create: build,
        update: (decorations, transaction) => getChunks(transaction.startState)?.chunks !== getChunks(transaction.state)?.chunks
            ? build(transaction.state) : decorations,
        provide: (field) => EditorView.decorations.from(field),
    });
}
