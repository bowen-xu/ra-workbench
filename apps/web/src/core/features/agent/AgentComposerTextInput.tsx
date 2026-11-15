import { memo, useLayoutEffect, useRef, useState, type RefObject } from "react";
import { EditorState, Compartment, Prec, Transaction } from "@codemirror/state";
import { drawSelection, EditorView, keymap, placeholder } from "@codemirror/view";
import { defaultKeymap, historyKeymap } from "@codemirror/commands";
import type { AgentInputDraft, AgentReferenceInput, AgentResourceScope } from "@research-assistant/core/agent";
import { composerBindingsEffect, composerEditorDraft, composerReferenceExtensions, composerTextChanges } from "./agentComposerEditor";
import { pasteComposerDraft, readComposerClipboard } from "./agentComposerClipboard";
import { AgentReferenceDocument } from "./AgentReferenceDocument";
import type { AgentResourceGateways } from "./agentResourceGateways";
import "./AgentComposerTextInput.css";
import { AgentEvidenceReferences } from "./AgentEvidenceReferences";
import { useComposerAutosize } from "./useComposerAutosize";

export interface AgentComposerInputHandle {
  readonly selectionStart: number; readonly selectionEnd: number;
  focus(): void; setSelectionRange(start: number, end: number): void;
  readonly element?: HTMLElement;
}
export interface AgentComposerKeyEvent {
  readonly key: string; readonly keyCode: number; readonly shiftKey: boolean; readonly ctrlKey: boolean;
  readonly metaKey: boolean; readonly altKey: boolean; readonly defaultPrevented: boolean;
  readonly nativeEvent: {readonly isComposing: boolean};
  preventDefault(): void; stopPropagation(): void;
}
interface Props {
  readonly plainText?: boolean;
  readonly inputRef: RefObject<AgentComposerInputHandle | null>;
  readonly draft: AgentInputDraft; readonly onDraftChange: (draft: AgentInputDraft) => void;
  readonly onChange: (text: string) => void; readonly onCaretChange: (text: string, caret: number) => void;
  readonly onKeyDown: (event: AgentComposerKeyEvent) => void;
  readonly "aria-label": string; readonly placeholder?: string; readonly rows?: number; readonly disabled?: boolean;
  readonly gateways?: AgentResourceGateways; readonly scope?: AgentResourceScope;
}

/** References replace their bound text in place. Rich mode remains mounted after
 * the last deletion, allowing undo to restore the token and its exact identity. */
export function AgentComposerTextInput(props: Props): React.JSX.Element {
  const committed = useRef(props);
  useLayoutEffect(() => {committed.current = props;});
  const [events] = useState(() => ({
    onChange: (value: string) => committed.current.onChange(value),
    onDraftChange: (draft: AgentInputDraft) => committed.current.onDraftChange(draft),
    onCaretChange: (text: string, caret: number) => committed.current.onCaretChange(text, caret),
    onKeyDown: (event: AgentComposerKeyEvent) => committed.current.onKeyDown(event),
  }));
  return <ComposerEditor {...props} {...events}/>;
}

function sameItems<T>(left: readonly T[] | undefined, right: readonly T[] | undefined): boolean {
  return left === right || !!left && !!right && left.length === right.length && left.every((item, index) => item === right[index]);
}

// Output updates may replace host callbacks and the draft envelope. Neither is
// an editor change. Stable event bridges always call the latest committed host;
// the memo boundary preserves composition, editor state and selection.
const ComposerEditor = memo(function ComposerEditor(props: Props): React.JSX.Element {
  const [rich, setRich] = useState(props.draft.bindings.length > 0);
  const [selected, setSelected] = useState<AgentReferenceInput | null>(null);
  const nativeDraft = useRef(props.draft);
  if (!rich && !props.draft.bindings.length) nativeDraft.current = props.draft;
  const mount = useRef<HTMLDivElement>(null), view = useRef<EditorView | null>(null);
  const latest = useRef(props); latest.current = props;
  const initialSelection = useRef({from: 0, to: 0, focused: false});
  const nativePaste = useRef<(ReturnType<typeof pasteComposerDraft> & {insert: string}) | null>(null);
  const configuration = useRef(new Compartment());
  useLayoutEffect(() => {
    if (!rich && props.draft.bindings.length) {
      const input = props.inputRef.current;
      initialSelection.current = {from: input?.selectionStart ?? 0, to: input?.selectionEnd ?? 0,
        focused: document.activeElement === input};
      setRich(true);
    }
  }, [rich, props.draft.bindings.length, props.inputRef]);
  useLayoutEffect(() => {
    if (!rich) {
      if (initialSelection.current.focused) { latest.current.inputRef.current?.focus(); initialSelection.current.focused = false; }
      return;
    }
    if (!mount.current) return;
    const current = latest.current;
    function config() { const p = latest.current; return [EditorView.contentAttributes.of({"aria-label": p["aria-label"], "data-tab-action": "both"}),
      EditorView.editable.of(!p.disabled), EditorState.readOnly.of(!!p.disabled), placeholder(p.placeholder ?? "")]; }
    const editor = new EditorView({parent: mount.current, state: EditorState.create({doc: nativeDraft.current.text,
      selection: {anchor: Math.min(initialSelection.current.from, nativeDraft.current.text.length), head: Math.min(initialSelection.current.to, nativeDraft.current.text.length)},
      extensions: [composerReferenceExtensions(setSelected), configuration.current.of(config()), EditorView.lineWrapping, drawSelection(),
        keymap.of([...defaultKeymap, ...historyKeymap]),
        Prec.highest(EditorView.domEventHandlers({keydown(event) {
          latest.current.onKeyDown({key: event.key, keyCode: event.keyCode, shiftKey: event.shiftKey, ctrlKey: event.ctrlKey,
            metaKey: event.metaKey, altKey: event.altKey, get defaultPrevented() { return event.defaultPrevented; },
            nativeEvent: event, preventDefault: () => event.preventDefault(), stopPropagation: () => event.stopPropagation()});
          if (event.key === "Tab" && !event.ctrlKey && !event.metaKey && !event.altKey) {
            event.preventDefault(); event.stopPropagation();
          }
          return event.defaultPrevented;
        }})),
        EditorView.updateListener.of(update => {
          const bindingsChanged = update.transactions.some(transaction => transaction.effects.some(effect => effect.is(composerBindingsEffect)));
          if (update.docChanged || bindingsChanged) latest.current.onDraftChange(composerEditorDraft(update.state));
          if (update.docChanged || update.selectionSet || bindingsChanged) latest.current.onCaretChange(update.state.doc.toString(), update.state.selection.main.head);
        }),
      ]})});
    if (nativePaste.current) {
      const paste = nativePaste.current; nativePaste.current = null;
      editor.dispatch({changes: {from: paste.from, to: paste.to, insert: paste.insert},
        effects: composerBindingsEffect.of(paste.draft.bindings), selection: {anchor: paste.caret}, userEvent: "input.paste"});
    } else if (nativeDraft.current.text !== current.draft.text) {
      editor.dispatch({changes: composerTextChanges(nativeDraft.current.text, current.draft.text),
        effects: composerBindingsEffect.of(current.draft.bindings), userEvent: "input.reference"});
    } else editor.dispatch({effects: composerBindingsEffect.of(current.draft.bindings), annotations: Transaction.addToHistory.of(false)});
    view.current = editor;
    const handle: AgentComposerInputHandle = {
      get selectionStart() { return editor.state.selection.main.from; }, get selectionEnd() { return editor.state.selection.main.to; },
      element: editor.dom, focus: () => editor.focus(),
      setSelectionRange(start, end) { editor.dispatch({selection: {anchor: Math.min(start, editor.state.doc.length), head: Math.min(end, editor.state.doc.length)}}); },
    };
    current.inputRef.current = handle;
    if (initialSelection.current.focused) editor.focus();
    return () => { if (current.inputRef.current === handle) current.inputRef.current = null; view.current = null; editor.destroy(); };
  }, [rich]);
  useLayoutEffect(() => {
    const editor = view.current;
    if (!editor) return;
    const p = props;
    editor.dispatch({effects: configuration.current.reconfigure([EditorView.contentAttributes.of({"aria-label": p["aria-label"], "data-tab-action": "both"}),
      EditorView.editable.of(!p.disabled), EditorState.readOnly.of(!!p.disabled), placeholder(p.placeholder ?? "")])});
  }, [props["aria-label"], props.disabled, props.placeholder, rich]);
  useLayoutEffect(() => {
    const editor = view.current;
    if (!editor) return;
    const p = props;
    if (JSON.stringify(composerEditorDraft(editor.state)) === JSON.stringify({text: p.draft.text, bindings: p.draft.bindings})) return;
    // Submitted/cleared text starts a new editing session; undo must not resend a previous message.
    if (!p.draft.text) {
      initialSelection.current.focused = editor.hasFocus;
      setRich(false); return;
    }
    editor.dispatch({changes: composerTextChanges(editor.state.doc.toString(), p.draft.text), effects: composerBindingsEffect.of(p.draft.bindings), userEvent: "input.reference"});
  }, [props.draft.text, props.draft.bindings, rich]);
  useComposerAutosize(props.inputRef, rich, props.draft.text, props.draft.evidence?.length ?? 0);
  return <><AgentEvidenceReferences evidence={props.draft.evidence ?? []} disabled={props.disabled ?? false} onChange={evidence => props.onDraftChange({...props.draft, evidence})} onClear={() => props.inputRef.current?.focus()}/>{rich ? <div ref={mount} className="agent-composer-inline-input" />
    : <textarea data-tab-action="both" ref={element => {props.inputRef.current = element;}} aria-label={props["aria-label"]} placeholder={props.placeholder} rows={props.rows}
      disabled={props.disabled} value={props.draft.text} onChange={event => {props.onChange(event.target.value); props.onCaretChange(event.target.value, event.target.selectionStart);}}
      onPaste={event => {
        if (props.disabled) return;
        if (props.plainText) return;
        const fragment = readComposerClipboard(event.clipboardData);
        if (!fragment) return;
        event.preventDefault();
        const input = event.currentTarget;
        const paste = pasteComposerDraft(props.draft, fragment, input.selectionStart, input.selectionEnd);
        nativeDraft.current = props.draft;
        initialSelection.current = {from: input.selectionStart, to: input.selectionEnd, focused: document.activeElement === input};
        nativePaste.current = {...paste, insert: fragment.text};
        setRich(true); props.onDraftChange(paste.draft); props.onCaretChange(paste.draft.text, paste.caret);
      }}
      onSelect={event => props.onCaretChange(event.currentTarget.value, event.currentTarget.selectionStart)} onKeyDown={event => {
        props.onKeyDown(event);
        if (event.key === "Tab" && !event.ctrlKey && !event.metaKey && !event.altKey) {event.preventDefault(); event.stopPropagation();}
      }} />}
    {selected && <AgentReferenceDocument reference={selected} onClose={() => {setSelected(null); props.inputRef.current?.focus();}}
      {...(props.gateways ? {gateways: props.gateways} : {})} {...(props.scope ? {scope: props.scope} : {})} />}</>;
}, (previous, next) => {
  const a = previous.draft, b = next.draft;
  return a.text === b.text && sameItems(a.bindings, b.bindings) && sameItems(a.evidence, b.evidence)
    && sameItems(a.unboundReferences, b.unboundReferences)
    && [...new Set([...Object.keys(previous), ...Object.keys(next)] as (keyof Props)[])]
      .every(key => key === "draft" || previous[key] === next[key]);
});
