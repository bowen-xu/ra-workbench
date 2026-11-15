import {isHistoryTransaction, redoDepth, undoDepth} from "@tiptap/pm/history";
import type {Transaction} from "@tiptap/pm/state";
import type {HbsHistoryEntry, HbsSession} from "@research-assistant/core/hbs";
import type {NoteDocumentV1} from "@research-assistant/core/notes";
import type {NoteEditorInstance} from "../../features/notes/editor/noteEditorSchema";
import {HbsOperationLog, operationTarget} from "./operationLog";

/** Passive capture uses native DOM events and TipTap's public transaction event,
 * including its ordered appended transactions. It never dispatches a command. */
export function captureNoteOperations(log: HbsOperationLog, editor: NoteEditorInstance, session: HbsSession<NoteDocumentV1>) {
  const context = {kind: "note", resourceId: session.getSnapshot().content.pageId, captureId: crypto.randomUUID()};
  const entries = new WeakMap<object, number>();
  let sequence = 0, batch = 0, detach: (() => void) | undefined, disposed = false;
  const record = (label: string, detail: unknown) => log.record(label, {...context, ...detail as object});
  const stopSession = log.observe(session, context.resourceId);
  function transactionData(transaction: Transaction) {
    const history = transaction.getMeta("addToHistory"), uiEvent = transaction.getMeta("uiEvent");
    return {time: transaction.time, history: isHistoryTransaction(transaction), docChanged: transaction.docChanged,
      selectionSet: transaction.selectionSet, selection: transaction.selection.toJSON(),
      addToHistory: typeof history === "boolean" ? history : null, uiEvent: typeof uiEvent === "string" ? uiEvent : null,
      appended: !!transaction.getMeta("appendedTransaction"),
      steps: transaction.steps.map(step => {
        const json = step.toJSON() as {stepType?: string; direction?: string; entry?: HbsHistoryEntry<NoteDocumentV1>};
        if (json.stepType !== "hbsSessionV1" || !json.entry) return json;
        const entry = json.entry;
        if (!entries.has(entry)) entries.set(entry, ++sequence);
        // The command owner may contain circular pending/cancelled intents.
        // Keep its identity and exact domain delta, never serialize that graph.
        return {stepType: json.stepType, direction: json.direction, entryId: entries.get(entry),
          committedBeforeVersion: entry.committedBefore.version, committedAfterVersion: entry.committedAfter?.version,
          contentChanges: session.adapter.diff(entry.before.content, entry.after.content),
          baselineChanges: session.adapter.diff(entry.before.baseline, entry.after.baseline)};
      })};
  }
  const transactions = ({transaction, appendedTransactions}: {transaction: Transaction; appendedTransactions: Transaction[]}) => {
    const ordered = [transaction, ...appendedTransactions];
    if (!ordered.some(item => item.docChanged || item.selectionSet || item.steps.length)) return;
    try {
      record("原生事务批次", {batch: ++batch, transactions: ordered.map(transactionData),
        undoDepth: undoDepth(editor.prosemirrorState), redoDepth: redoDepth(editor.prosemirrorState),
        version: session.getSnapshot().observed.version});
    } catch { record("原生事务记录失败", {code: "MODEL_OPERATION_TRANSACTION_UNAVAILABLE"}); }
  };
  function attach() {
    if (disposed || !editor.domElement || detach) return;
    const root = editor.domElement;
    record("原生编辑器连接", {document: editor.prosemirrorState.doc.toJSON(), selection: editor.prosemirrorState.selection.toJSON(),
      undoDepth: undoDepth(editor.prosemirrorState), redoDepth: redoDepth(editor.prosemirrorState)});
    const input = (event: Event) => {
      if (!(event.target instanceof globalThis.Node) || !root.contains(event.target)) return;
      const value = event as InputEvent;
      record(event.type === "beforeinput" ? "输入前事件" : "文档输入／删除事件",
        {inputType: value.inputType, data: value.data, composing: value.isComposing, target: operationTarget(event.target)});
    };
    const key = (event: KeyboardEvent) => {
      if (!(event.target instanceof globalThis.Node) || !root.contains(event.target)) return;
      const history = (event.ctrlKey || event.metaKey) && (event.key.toLowerCase() === "z" || event.key.toLowerCase() === "y");
      const redo = event.shiftKey || event.key.toLowerCase() === "y";
      record(history ? redo ? "触发重做" : "触发撤销" : "按键事件", {key: event.key, code: event.code,
        ctrl: event.ctrlKey, meta: event.metaKey, alt: event.altKey, shift: event.shiftKey, repeat: event.repeat,
        composing: event.isComposing, target: operationTarget(event.target)});
    };
    const paste = (event: ClipboardEvent) => record("粘贴事件", {
      plainText: event.clipboardData?.getData("text/plain") ?? "", html: event.clipboardData?.getData("text/html") ?? "",
      target: operationTarget(event.target)});
    const composition = (event: CompositionEvent) => record(event.type === "compositionstart" ? "开始组合输入" : "结束组合输入",
      {data: event.data, target: operationTarget(event.target)});
    // The shared history scope handles Undo at window capture and stops its
    // propagation. Observe there too, restricted to this editor's own target.
    window.addEventListener("beforeinput", input, true); root.addEventListener("input", input, true);
    window.addEventListener("keydown", key, true); root.addEventListener("paste", paste, true);
    root.addEventListener("compositionstart", composition, true); root.addEventListener("compositionend", composition, true);
    editor._tiptapEditor.on("transaction", transactions);
    detach = () => {
      window.removeEventListener("beforeinput", input, true); root.removeEventListener("input", input, true);
      window.removeEventListener("keydown", key, true); root.removeEventListener("paste", paste, true);
      root.removeEventListener("compositionstart", composition, true); root.removeEventListener("compositionend", composition, true);
      editor._tiptapEditor.off("transaction", transactions);
    };
  }
  editor._tiptapEditor.on("mount", attach); attach();
  return () => {disposed = true; detach?.(); stopSession(); editor._tiptapEditor.off("mount", attach);};
}
