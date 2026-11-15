import { useLayoutEffect, useRef, useState } from "react";
import { Compartment, EditorState, Transaction } from "@codemirror/state";
import { EditorView, drawSelection, keymap, lineNumbers } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { getChunks, getOriginalDoc, originalDocChangeEffect, unifiedMergeView } from "@codemirror/merge";
import { chunkActionDecorations } from "./codeMirrorChunkActions";
import { resolveTextChunk, resolveTextDiff, textChanges } from "./textDiffCommands";
import { EditorDiffControls } from "./EditorDiffControls";
import "./TextDiffEditor.css";
import { EditorDiffHighlight, type EditorDiffHighlightAppearance } from "./EditorDiffHighlight";
/** The writing editor's exact merge engine, chunk widgets, commands and toolbar.
 * Default read-only prevents typing; explicit review actions can still save D/B. */
export function TextDiffEditor({ content, baseline, editable = false, disabled = false, onChange, onReview, syncVersion = 0, revealFirstChange = false, label = "文档正文", appearance }: {
    readonly appearance?: EditorDiffHighlightAppearance;
    readonly revealFirstChange?: boolean;
    readonly syncVersion?: number;
    readonly content: string;
    readonly baseline: string;
    readonly editable?: boolean;
    readonly disabled?: boolean;
    readonly onChange?: (text: string) => void;
    readonly onReview?: (value: {
        content: string;
        baseline: string;
    }) => void;
    readonly label?: string;
}) {
    const mount = useRef<HTMLDivElement>(null), view = useRef<EditorView | null>(null), latest = useRef({ onChange, onReview, disabled });
    latest.current = { onChange, onReview, disabled };
    const actions = useRef(new Compartment());
    const mode = useRef(new Compartment()), external = useRef(false);
    const [status, setStatus] = useState({ count: 0, active: 1 });
    function publish(editor: EditorView) { latest.current.onReview?.({ content: editor.state.doc.toString(), baseline: getOriginalDoc(editor.state).toString() }); }
    const review = (editor: EditorView, position: number, action: "keep" | "undo") => { if (!latest.current.disabled && latest.current.onReview && resolveTextChunk(editor, position, action))
        publish(editor); };
    useLayoutEffect(() => {
        if (!mount.current)
            return;
        const editor = new EditorView({ parent: mount.current, state: EditorState.create({ doc: content, extensions: [
                    history(), drawSelection(), lineNumbers(), EditorView.lineWrapping,
                    mode.current.of([EditorView.editable.of(editable && !disabled), keymap.of(editable && !disabled ? [...defaultKeymap, ...historyKeymap] : [])]),
                    unifiedMergeView({ original: baseline, mergeControls: false, syntaxHighlightDeletions: false, allowInlineDiffs: false, diffConfig: { scanLimit: 1000, timeout: 50 } }),
                    actions.current.of(chunkActionDecorations(review, disabled || !onReview)), EditorView.contentAttributes.of({ "aria-label": label }),
                    EditorView.updateListener.of(update => {
                        const chunks = getChunks(update.state)?.chunks ?? [];
                        const cursor = update.state.selection.main.head;
                        setStatus({ count: chunks.length, active: Math.max(1, chunks.findIndex(c => c.fromB <= cursor && cursor <= c.endB) + 1) });
                        if (update.docChanged && !external.current)
                            latest.current.onChange?.(update.state.doc.toString());
                    }),
                ] }) });
        view.current = editor;
        const first=getChunks(editor.state)?.chunks[0];
        if(revealFirstChange && first) {const position=Math.min(first.fromB,editor.state.doc.length);editor.dispatch({selection:{anchor:position},effects:EditorView.scrollIntoView(position,{y:"center"})});}
        setStatus({ count: getChunks(editor.state)?.chunks.length ?? 0, active: 1 });
        return () => { view.current = null; editor.destroy(); };
    }, []);
    useLayoutEffect(() => { view.current?.dispatch({ effects: actions.current.reconfigure(chunkActionDecorations(review, disabled || !onReview)) }); }, [disabled, !!onReview]);
    useLayoutEffect(() => { view.current?.dispatch({ effects: mode.current.reconfigure([EditorView.editable.of(editable && !disabled), keymap.of(editable && !disabled ? [...defaultKeymap, ...historyKeymap] : [])]) }); }, [editable, disabled]);
    useLayoutEffect(() => {
        const editor = view.current;
        if (!editor)
            return;
        external.current = true;
        try {
            const old = editor.state.doc.toString(), before = getOriginalDoc(editor.state).toString();
            if (old !== content || before !== baseline)
                editor.dispatch({ changes: textChanges(old, content), effects: originalDocChangeEffect(editor.state, textChanges(before, baseline)), annotations: Transaction.addToHistory.of(false) });
        }
        finally {
            external.current = false;
        }
    }, [content, baseline, syncVersion]);
    return <EditorDiffHighlight appearance={appearance}><div className="text-diff-editor"><EditorDiffControls count={status.count} active={status.active} disabled={disabled || !onReview} onMove={direction => { const editor = view.current; if (!editor)
        return; const chunks = getChunks(editor.state)?.chunks ?? []; const index = Math.min(chunks.length - 1, Math.max(0, status.active - 1 + (direction === "next" ? 1 : -1))); const chunk = chunks[index]; if (chunk)
        editor.dispatch({ selection: { anchor: chunk.fromB }, scrollIntoView: true }); }} onResolve={(action, scope) => {
            const editor = view.current;
            if (!editor || disabled || !onReview)
                return;
            if (scope === "all") {
                if (resolveTextDiff(editor, action))
                    publish(editor);
            }
            else {
                const chunk = getChunks(editor.state)?.chunks[status.active - 1];
                if (chunk && resolveTextChunk(editor, chunk.fromB, action))
                    publish(editor);
            }
        }}/>
  <div className="text-diff-editor-content" ref={mount}/></div></EditorDiffHighlight>;
}
