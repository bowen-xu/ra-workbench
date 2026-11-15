import { ControlHint } from "../ControlHint";
import { ActionMenu, ActionMenuItem } from "../ActionMenu";
import { revealTextDocument, saveTextDocumentAs } from "./textDocumentFileActions";
import { notify } from "../../services/notifications";
import { TextEditorPath } from "./TextEditorPath";
import { Fragment, useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { getChunks, getOriginalDoc, originalDocChangeEffect } from "@codemirror/merge";
import { EditorView } from "@codemirror/view";
import { Compartment, StateEffect, Transaction, type EditorState } from "@codemirror/state";
import { isolateHistory } from "@codemirror/commands";
import { ChevronDown, FolderOpen, Save, Search } from "lucide-react";
import { openSearchPanel } from "@codemirror/search";
import { EditorDiffControls } from "../editor-diff/EditorDiffControls";
import { resolveTextChunk, resolveTextDiff, textChanges, textChunkResolution, textDiffResolution } from "../editor-diff/textDiffCommands";
import { chunkActionDecorations } from "../editor-diff/codeMirrorChunkActions";
import { createTextEditorState } from "./textEditorState";
import { textEditorInlinePanels } from "./textEditorInlinePanels";
import { incomingDecorations, incomingGroups, resolveIncomingGroup, resolveIncomingReview, type IncomingGroup, type IncomingResolution, type TextEditorIncoming } from "./textEditorIncoming";
import "./TextEditor.css";
import { baselineHistoryChange, editorHistoryAction, type TextEditorHistoryPort } from "./textEditorHistory";
import { EditorDiffChunkActions } from "../editor-diff/EditorDiffChunkButtons";
import type { EditorReviewConfiguration } from "../editor-diff/EditorReviewActions";
import { EditorDiffHighlight } from "../editor-diff/EditorDiffHighlight";

export interface TextEditorProps {
  readonly documentId: string;
  readonly path: string;
  readonly value: string;
  readonly baseline?: string;
  /** Prose keeps native input/review/history with no document chrome. */
  readonly presentation?: "document" | "prose";
  readonly readOnly?: boolean;
  /** Read-only DOM may own structural action history. Ordinary text mutations
   * remain rejected; only controlled installs and native history may replay. */
  readonly readOnlyHistory?: boolean;
  readonly dirty?: boolean;
  readonly saving?: boolean;
  readonly onChange: (value: string, baseline: string) => void;
  readonly onSave?: () => void;
  /** A controlled host can persist the planned review before installing it. */
  readonly onReview?: (value: string, baseline: string) => void;
  readonly reviewDisabled?: boolean;
  readonly reviewConfiguration?: EditorReviewConfiguration;
  readonly incoming?: readonly TextEditorIncoming[];
  readonly onIncomingResolve?: (resolution: IncomingResolution) => void;
  /** Reconcile domain context with CM history; persistence stays in the host. */
  readonly history?: TextEditorHistoryPort;
  /** Allows enclosing structured cards to record metadata decisions in this
   * same editor history, including decisions without text changes. */
  readonly historyActions?: { readonly bind: (run: (action: () => void) => void) => () => void };
  /** Optional resource-owned retention across host handoffs. It stores native
   * CM state, never a second history stack or a serialized domain snapshot. */
  readonly stateRetention?: { readonly load:()=>EditorState|undefined;readonly save:(state:EditorState)=>void };
  readonly inlinePanels?: readonly { readonly id: string; readonly at: number; readonly content: ReactNode }[];
  readonly fileActions?: { readonly reveal: () => Promise<void>; readonly saveAs: (content: string) => Promise<void> };
}

/** One mounted instance per document preserves history, selection and scroll between tabs. */
export function TextEditor(props: TextEditorProps): React.JSX.Element {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const latest = useRef(props); latest.current = props;
  const [reviewCompartment] = useState(() => new Compartment());
  const [panelCompartment] = useState(() => new Compartment());
  const [incomingCompartment] = useState(() => new Compartment());
  const incomingHosts = useRef(new Map<string, HTMLElement>());
  const incomingRef = useRef<readonly IncomingGroup[]>([]);
  const [incoming, setIncoming] = useState<readonly IncomingGroup[]>([]);
  const panelHosts = useRef(new Map<string, HTMLDivElement>());
  const panels = (props.inlinePanels ?? []).map(panel => {
    let element = panelHosts.current.get(panel.id);
    if (!element) { element = document.createElement("div"); element.className = "text-editor-inline-panel"; panelHosts.current.set(panel.id, element); }
    return { ...panel, element };
  });
  const [review, setReview] = useState({ count: 0, active: 0 });
  function refresh(editor: EditorView): void {
    const chunks = getChunks(editor.state)?.chunks ?? [];
    const index = chunks.findIndex(chunk => chunk.endB >= editor.state.selection.main.head);
    const active = chunks.length ? (index < 0 ? chunks.length : index + 1) : 0;
    setReview(previous => previous.count === chunks.length && previous.active === active ? previous : { count: chunks.length, active });
  }
  useEffect(() => {
    const retainedState=latest.current.stateRetention?.load();
    const editor = new EditorView({ parent: host.current!, state: createTextEditorState({
      content: latest.current.value, baseline: latest.current.baseline ?? latest.current.value,
      readOnly: latest.current.readOnly === true, path: latest.current.path,
      readOnlyHistory: latest.current.readOnlyHistory === true,
      presentation: latest.current.presentation ?? "document",
      isReviewDisabled: () => latest.current.saving === true || latest.current.reviewDisabled === true,
      reviewCompartment,
      reviewDisabled: latest.current.saving === true || latest.current.reviewDisabled === true,
      onResolveReview: resolveChunk,
      historyEnabled: !!latest.current.history,
      ...(retainedState?{retainedState}:{}),
      onHistoryRestore: update => {
        const effects = update.transactions.filter(transaction => transaction.isUserEvent("undo") || transaction.isUserEvent("redo")).flatMap(transaction => transaction.effects);
        const replay = effects.filter(effect => effect.is(editorHistoryAction)).at(-1);
        if (replay) { replay.value.after.restore(update.state.doc.toString(), getOriginalDoc(update.state).toString()); return true; }
        const baselines = effects.filter(effect => effect.is(baselineHistoryChange)).map(effect => effect.value);
        if (!baselines.length || !latest.current.history?.restoreEdit) return false;
        latest.current.history.restoreEdit(update.state.doc.toString(), baselines); return true;
      },
      onChange: (content, baseline) => latest.current.onChange(content, baseline), onUpdate: update => {
        refresh(update.view);
        if (update.transactions.some(transaction => transaction.isUserEvent("review"))) latest.current.onSave?.();
      },
    }) });
    editor.dispatch({ effects: StateEffect.appendConfig.of([panelCompartment.of([]), incomingCompartment.of([])]) });
    view.current = editor; refresh(editor);
    return () => { latest.current.stateRetention?.save(editor.state); view.current = null; editor.destroy(); };
  }, [props.documentId, props.readOnly, props.readOnlyHistory, props.presentation]);
  useEffect(() => {
    const editor = view.current;
    if (!editor) return;
    const changed = editor.state.doc.toString() !== props.value;
    const baselineChanged = props.baseline !== undefined && getOriginalDoc(editor.state).toString() !== props.baseline;
    if (changed || baselineChanged) editor.dispatch({
      ...(changed ? { changes: textChanges(editor.state.doc.toString(), props.value) } : {}),
      effects: baselineChanged ? originalDocChangeEffect(editor.state, textChanges(getOriginalDoc(editor.state).toString(), props.baseline!)) : [],
      userEvent: "input.agent", annotations: props.history ? Transaction.addToHistory.of(false) : [],
    });
  }, [props.value, props.baseline]);
  useEffect(() => {
    const editor = view.current;
    if (editor) return props.historyActions?.bind(action => performHistory(editor,action));
  }, [props.historyActions, props.documentId]);
  useEffect(() => {
    const editor = view.current;
    if (!editor) return;
    const groups = incomingGroups(editor.state, props.incoming ?? [], incomingHosts.current);
    incomingRef.current = groups; setIncoming(groups);
    editor.dispatch({ effects: [reviewCompartment.reconfigure(chunkActionDecorations(resolveChunk,
      props.readOnly === true || props.saving === true || props.reviewDisabled === true, groups, props.reviewConfiguration?.labels)),
      incomingCompartment.reconfigure(incomingDecorations(editor.state, groups))] });
    const retained = new Set(groups.flatMap(group => [`${group.id}:incoming`, `${group.id}:actions`, `${group.id}:original`]));
    for (const id of incomingHosts.current.keys()) if (!retained.has(id)) incomingHosts.current.delete(id);
  }, [props.value, props.baseline, props.incoming, props.readOnly, props.saving, props.reviewDisabled, props.reviewConfiguration, props.documentId, reviewCompartment, incomingCompartment]);
  useEffect(() => {
    const editor = view.current;
    if (!editor) return;
    editor.dispatch({ effects: panelCompartment.reconfigure(textEditorInlinePanels(editor.state, panels)) });
    const retained = new Set(panels.map(panel => panel.id));
    for (const id of panelHosts.current.keys()) if (!retained.has(id)) panelHosts.current.delete(id);
  }, [props.inlinePanels, props.documentId, props.readOnly, panelCompartment]);
  function resolveChunk(editor: EditorView, position: number, action: "keep" | "undo"): void {
    if (latest.current.readOnly || latest.current.saving || latest.current.reviewDisabled || editor.composing) return;
    const conflict = incomingRef.current.find(group => group.from <= position && position <= group.to);
    if (conflict && latest.current.onIncomingResolve) { resolveIncoming(editor, resolveIncomingGroup(editor.state, conflict, action)); return; }
    if (!latest.current.onReview) { resolveTextChunk(editor, position, action); return; }
    const transaction = textChunkResolution(editor.state, position, action);
    if (transaction) {
      const next = editor.state.update(transaction).state;
      const planned = { value: next.doc.toString(), baseline: getOriginalDoc(next).toString() };
      performHistory(editor, () => latest.current.onReview!(planned.value, planned.baseline), planned);
    }
  }
  function resolve(action: "keep" | "undo", scope: "current" | "all"): void {
    const editor = view.current;
    if (!editor) return;
    if (latest.current.readOnly || latest.current.saving || latest.current.reviewDisabled) return;
    if (scope === "all" && latest.current.onReview) {
      if (editor.composing) return;
      if (incomingRef.current.length && latest.current.onIncomingResolve) {
        resolveIncoming(editor, resolveIncomingReview(editor.state, incomingRef.current, action)); return;
      }
      const transaction = textDiffResolution(editor.state, action);
      if (transaction) {
        const next = editor.state.update(transaction).state;
        const planned = { value: next.doc.toString(), baseline: getOriginalDoc(next).toString() };
        performHistory(editor, () => latest.current.onReview!(planned.value, planned.baseline), planned);
      }
    } else if (scope === "all") resolveTextDiff(editor, action);
    else {
      const chunk = getChunks(editor.state)?.chunks[review.active - 1];
      if (chunk) resolveChunk(editor, chunk.fromB, action);
    }
  }
  function performHistory(editor: EditorView, action: () => void, review?: { value: string; baseline: string }): void {
    if (!latest.current.history) { action(); return; }
    const entry = latest.current.history.perform(action, review);
    if (!entry) return;
    editor.dispatch({ changes: textChanges(editor.state.doc.toString(), entry.after.value),
      effects: [...(getOriginalDoc(editor.state).toString() === entry.after.baseline ? []
        : [originalDocChangeEffect(editor.state, textChanges(getOriginalDoc(editor.state).toString(), entry.after.baseline))]), editorHistoryAction.of(entry)],
      annotations: [isolateHistory.of("full"), Transaction.userEvent.of("restore.action")],
    });
    if (!latest.current.readOnlyHistory) editor.focus();
  }
  function resolveIncoming(editor: EditorView, resolution: IncomingResolution): void {
    const review = resolution.action === "keep" || resolution.action === "undo" ? resolution : undefined;
    performHistory(editor, () => latest.current.onIncomingResolve?.(resolution), review);
  }
  const prose = props.presentation === "prose";
  return <EditorDiffHighlight appearance={props.reviewConfiguration?.highlight}><section className={`text-editor${prose ? " text-editor-prose" : ""}`} aria-label={`${prose ? "字段正文" : "文本编辑器"} ${props.path}`} onKeyDownCapture={event => {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") { event.preventDefault(); if (!props.readOnly && !props.saving) props.onSave?.(); }
  }}>
    {!prose && <header className="text-editor-toolbar">
      <TextEditorPath path={props.path} />
      <ControlHint content="在当前文件中查找（Cmd/Ctrl+F）"><button type="button" aria-label="查找文本" onClick={() => { if (view.current) openSearchPanel(view.current); }}><Search size={16} /></button></ControlHint>
      <ActionMenu className="text-editor-file-menu" triggerClassName="text-editor-file-menu-trigger" contentClassName="text-editor-file-menu-content" label="文本文件操作" trigger={<ChevronDown size={16} />} portal hint>
        <ActionMenuItem onClick={() => { void (props.fileActions?.reveal() ?? revealTextDocument(props.documentId, props.path)).catch(reason => notify.error(reason instanceof Error ? reason.message : String(reason))); }}><FolderOpen size={16} />在 Finder 中显示</ActionMenuItem>
        <ActionMenuItem onClick={() => { const content = view.current?.state.doc.toString() ?? props.value; void (props.fileActions?.saveAs(content) ?? saveTextDocumentAs(props.path, content)).catch(reason => notify.error(reason instanceof Error ? reason.message : String(reason))); }}><Save size={16} />另存为…</ActionMenuItem>
      </ActionMenu>
    </header>}
    {!prose && <EditorDiffControls {...review} showBulk={props.reviewConfiguration?.showBulk ?? true} disabled={props.readOnly === true || props.saving === true || props.reviewDisabled === true} onResolve={resolve} onMove={direction => {
      const editor = view.current;
      if (!editor) return;
      const chunks = getChunks(editor.state)?.chunks ?? [];
      const index = (review.active - 1 + (direction === "next" ? 1 : -1) + chunks.length) % chunks.length;
      const chunk = chunks[index];
      if (chunk) { editor.dispatch({ selection: { anchor: Math.min(chunk.fromB, editor.state.doc.length) }, effects: EditorView.scrollIntoView(Math.min(chunk.fromB, editor.state.doc.length), { y: "center" }) }); editor.focus(); }
    }} />}
    <div className="text-editor-surface" ref={host} />
    {incoming.map(group => <Fragment key={group.id}>
      {!group.chunk && createPortal(<div>{group.original.replace(/\n$/u, "") || "（删除）"}</div>, group.originalHost)}
      {createPortal(<div role="region" aria-label="incoming">{props.reviewConfiguration?.renderIncoming?.(group.incoming) ?? (group.incoming.replace(/\n$/u, "") || "（删除）")}</div>, group.incomingHost)}
      {createPortal(<EditorDiffChunkActions conflict configuration={props.reviewConfiguration} disabled={props.readOnly === true || props.saving === true || props.reviewDisabled === true} onResolve={action => {
        const editor = view.current;
        if (editor && !latest.current.readOnly && !latest.current.saving && !latest.current.reviewDisabled && !editor.composing) resolveIncoming(editor, resolveIncomingGroup(editor.state, group, action));
      }} />, group.actionsHost)}
    </Fragment>)}
    {panels.map(panel => createPortal(panel.content, panel.element, panel.id))}
    {!prose && <footer>{props.readOnly ? "只读" : props.saving ? "保存中…" : props.dirty ? "未保存" : "已保存"}<span>UTF-8 · {props.value.split("\n").length} 行{review.count ? ` · ${review.count} 处待审变更` : ""}</span></footer>}
  </section></EditorDiffHighlight>;
}
