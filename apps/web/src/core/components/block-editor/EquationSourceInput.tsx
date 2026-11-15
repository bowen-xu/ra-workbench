import { useLayoutEffect, useRef, type RefObject } from "react";
import { EditorSelection, EditorState, Prec, Transaction } from "@codemirror/state";
import { EditorView, drawSelection, keymap, placeholder, tooltips } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, insertNewline } from "@codemirror/commands";
import { acceptCompletion, clearSnippet, closeBrackets, closeBracketsKeymap,
  closeCompletion, completionKeymap, completionStatus, nextSnippetField, prevSnippetField, snippetKeymap } from "@codemirror/autocomplete";
import { bracketMatching, defaultHighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { useOverlayTarget } from "../overlay/OverlayPortal";
import { latexLanguage } from "../text-editor/latexLanguage";
import { latexMathCompletionSource } from "../text-editor/latexMathCompletion";
import { virtualCompletion } from "../text-editor/virtualCompletion";
import "./EquationSourceInput.css";

export interface EquationSourceInputHandle {
  focus(): void;
  select(): void;
  ownsTarget(target: EventTarget | null): boolean;
}
interface Props {
  readonly inputRef: RefObject<EquationSourceInputHandle | null>;
  readonly value: string;
  readonly onChange: (source: string) => void;
  readonly onComplete: () => void;
  readonly onCancel: () => void;
  readonly onMoveOut?: (side: "left" | "right") => void;
  readonly "aria-label": string;
  readonly placeholder: string;
}

/** A small LaTeX editor; the scroller grows naturally up to its CSS height cap. */
export function EquationSourceInput(props: Props): React.JSX.Element {
  const mount = useRef<HTMLDivElement>(null), view = useRef<EditorView | null>(null);
  const latest = useRef(props); latest.current = props;
  const target = useOverlayTarget("popup", mount);
  useLayoutEffect(() => {
    if (!mount.current) return;
    const moveOut = (editor: EditorView, side: "left" | "right") => {
      const selection = editor.state.selection.main;
      if (!latest.current.onMoveOut || !selection.empty
        || selection.head !== (side === "left" ? 0 : editor.state.doc.length)) return false;
      latest.current.onMoveOut(side); return true;
    };
    const closeEditing = (editor: EditorView) => {
      const visible = completionStatus(editor.state) === "active";
      closeCompletion(editor);
      if (clearSnippet(editor) || visible) return true;
      latest.current.onCancel(); return true;
    };
    const acceptOrTab = (editor: EditorView) => {
      if (acceptCompletion(editor) || nextSnippetField(editor)) return true;
      editor.dispatch(editor.state.replaceSelection("\t"), { scrollIntoView: true, userEvent: "input" });
      return true;
    };
    const editor = new EditorView({ parent: mount.current, state: EditorState.create({ doc: latest.current.value,
      extensions: [latexLanguage(), closeBrackets(), bracketMatching(), history(), drawSelection(), EditorView.lineWrapping,
        syntaxHighlighting(defaultHighlightStyle), placeholder(latest.current.placeholder),
        EditorView.contentAttributes.of({ "aria-label": latest.current["aria-label"], "aria-multiline": "true", "data-tab-action": "both", spellcheck: "false" }),
        tooltips({ parent: target, position: "fixed" }),
        snippetKeymap.of([]),
        // As in the writing editor, Escape must precede autocomplete's own
        // highest-priority binding so an invisible pending query cannot eat it.
        Prec.highest(keymap.of([
          { key: "Escape", run: closeEditing },
          { key: "Tab", run: acceptOrTab, shift: prevSnippetField, preventDefault: true },
          { key: "Enter", run: editor => { if (!acceptCompletion(editor)) latest.current.onComplete(); return true; } },
          { key: "Shift-Enter", run: insertNewline },
          { key: "ArrowLeft", run: editor => moveOut(editor, "left") },
          { key: "ArrowRight", run: editor => moveOut(editor, "right") },
        ])),
        virtualCompletion({ override: [latexMathCompletionSource], activateOnTyping: true, interactionDelay: 0,
          tooltipClass: () => "equation-source-completions" }),
        keymap.of([...closeBracketsKeymap, ...defaultKeymap, ...historyKeymap, ...completionKeymap]),
        EditorState.transactionFilter.of(transaction => transaction.newDoc.length > 32_768
          && transaction.newDoc.length > transaction.startState.doc.length ? [] : transaction),
        EditorView.domEventHandlers({ keydown(event) { event.stopPropagation(); return false; } }),
        EditorView.updateListener.of(update => { if (update.docChanged) latest.current.onChange(update.state.doc.toString()); }),
      ],
    }) });
    view.current = editor;
    const handle: EquationSourceInputHandle = {
      focus: () => editor.contentDOM.focus({ preventScroll: true }),
      select: () => editor.dispatch({ selection: EditorSelection.range(0, editor.state.doc.length) }),
      ownsTarget: candidate => candidate instanceof Node && (editor.dom.contains(candidate) || target.contains(candidate)),
    };
    latest.current.inputRef.current = handle;
    // IME candidate keys must not reach completion or equation commit bindings.
    const ignoreComposition = (event: KeyboardEvent) => {
      if (event.isComposing || event.keyCode === 229) event.stopImmediatePropagation();
    };
    editor.contentDOM.addEventListener("keydown", ignoreComposition, true);
    return () => {
      editor.contentDOM.removeEventListener("keydown", ignoreComposition, true);
      if (latest.current.inputRef.current === handle) latest.current.inputRef.current = null;
      view.current = null; editor.destroy();
    };
  }, [target]);
  useLayoutEffect(() => {
    const editor = view.current;
    if (!editor || editor.state.doc.toString() === props.value) return;
    editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: props.value },
      annotations: Transaction.addToHistory.of(false) });
  }, [props.value]);
  return <div ref={mount} className="equation-source-input" />;
}
