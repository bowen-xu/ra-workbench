import { closeHistory, isHistoryTransaction } from "@tiptap/pm/history";
import { Plugin, PluginKey, type EditorState, type Transaction } from "@tiptap/pm/state";
import { Step, StepResult, type Mappable } from "@tiptap/pm/transform";
import type { Node, Schema } from "@tiptap/pm/model";
import { decodeNoteDocument, type NoteDocumentV1 } from "@research-assistant/core/notes";
import { shouldSynchronizeReviewBaseline } from "@research-assistant/core/document-review";

export type NoteReviewOrigin = "agent" | "review" | "manual" | "restore";
export interface NoteReviewHistoryState {
  readonly baseline: NoteDocumentV1;
  readonly revision: number;
  readonly origin: NoteReviewOrigin;
  /** Proof identifies the original durable review request, never a new replay. */
  readonly historyProofRequestId?: string;
  /** Local installation identity; it becomes a durable edit receipt only on
   * the first undo. It is not itself a native history proof. */
  readonly installationId?: string;
  readonly direction?: "undo" | "redo";
}
export type NoteReviewBaselineSynchronizer = (before: EditorState, after: EditorState,
  baseline: NoteDocumentV1) => NoteDocumentV1 | undefined;
interface BaselineStepValue {
  readonly before: NoteDocumentV1;
  readonly after: NoteDocumentV1;
  readonly origin: NoteReviewOrigin;
  readonly requestId?: string;
  readonly installationId?: string;
  readonly direction?: "undo" | "redo";
}
const baselineKey = new PluginKey<NoteReviewHistoryState>("note-review-history");
export const NOTE_REVIEW_ORIGIN = "note-review-origin";
const STEP_ID = "researchAssistantNoteReviewBaselineV1";
const ORIGINS = new Set<NoteReviewOrigin>(["agent", "review", "manual", "restore"]);

function snapshot(value: unknown): NoteDocumentV1 {
  const encoded = JSON.stringify(value);
  const decoded = decodeNoteDocument(JSON.parse(encoded));
  const canonical = (input: unknown) => JSON.stringify(input, (_key, item: unknown) => item && typeof item === "object" && !Array.isArray(item)
    ? Object.fromEntries(Object.entries(item).sort(([a], [b]) => a.localeCompare(b))) : item);
  if (canonical(value) !== canonical(decoded)) throw new Error("笔记审阅基线无法无损恢复");
  return decoded;
}

/** A document-preserving Step makes baseline-only Keep reversible in the
 * editor's existing ProseMirror history. Metadata-only transactions are not
 * recorded by that history. No second undo stack or document replacement. */
class NoteReviewBaselineStep extends Step {
  constructor(readonly value: BaselineStepValue) { super(); }
  apply(doc: Node): StepResult { return StepResult.ok(doc); }
  invert(): Step {
    return new NoteReviewBaselineStep({ ...this.value, before: this.value.after, after: this.value.before,
      direction: this.value.direction === "undo" ? "redo" : "undo" });
  }
  // Baselines contain stable block identities, not ProseMirror positions.
  map(_mapping: Mappable): Step { return this; }
  toJSON(): unknown { return { stepType: STEP_ID, ...this.value }; }
  static override fromJSON(_schema: Schema, input: unknown): Step {
    if (!input || typeof input !== "object" || Array.isArray(input)) throw new Error("无效的笔记审阅历史");
    const value = input as Record<string, unknown>;
    if (Object.keys(value).some(key => !["stepType", "before", "after", "origin", "requestId", "installationId", "direction"].includes(key))
      || !ORIGINS.has(value.origin as NoteReviewOrigin)
      || value.requestId !== undefined && (typeof value.requestId !== "string" || !value.requestId.length)
      || value.installationId !== undefined && (value.origin !== "agent" || typeof value.installationId !== "string" || !value.installationId.length)
      || value.direction !== undefined && value.direction !== "undo" && value.direction !== "redo") throw new Error("无效的笔记审阅历史");
    const before = snapshot(value.before), after = snapshot(value.after);
    if (before.pageId !== after.pageId) throw new Error("笔记审阅历史不能跨页");
    return new NoteReviewBaselineStep({ before, after, origin: value.origin as NoteReviewOrigin,
      ...(value.requestId === undefined ? {} : { requestId: value.requestId as string }),
      ...(value.installationId === undefined ? {} : { installationId: value.installationId as string }),
      ...(value.direction === undefined ? {} : { direction: value.direction as "undo" | "redo" }) });
  }
}
Step.jsonID(STEP_ID, NoteReviewBaselineStep);

/** The field remains installed when B equals D, so undoing Keep restores its
 * baseline in the same editor instance. The Notes host owns durable D/B saves. */
export function createNoteReviewHistory(initialBaseline: NoteDocumentV1,
  onChange?: (value: NoteReviewHistoryState) => void,
  syncBaseline?: NoteReviewBaselineSynchronizer,
  canReplayHistory: () => boolean = () => true): Plugin<NoteReviewHistoryState> {
  const baseline = snapshot(initialBaseline);
  const reviewReplays = new WeakSet<EditorState>();
  return new Plugin<NoteReviewHistoryState>({
    key: baselineKey,
    filterTransaction: transaction => !isHistoryTransaction(transaction) || canReplayHistory(),
    state: {
      init: () => ({ baseline, revision: 0, origin: "restore" }),
      apply(transaction, previous, before, after) {
        let next = previous;
        let hasBaselineStep = false;
        for (const step of transaction.steps) if (step instanceof NoteReviewBaselineStep) {
          // Review decisions may have been clicked far from the stored caret.
          // Preserve reading position only for their undo/redo, leaving normal
          // text history and explicit navigation's scrolling unchanged.
          if (step.value.origin === "review" && isHistoryTransaction(transaction)) reviewReplays.add(after);
          hasBaselineStep = true;
          if (step.value.after.pageId !== baseline.pageId) throw new Error("笔记审阅历史不能跨页");
          next = { baseline: step.value.after, revision: next.revision + 1, origin: step.value.origin,
            ...(step.value.requestId === undefined ? {} : { historyProofRequestId: step.value.requestId }),
            ...(step.value.installationId === undefined ? {} : { installationId: step.value.installationId }),
            ...(step.value.direction === undefined ? {} : { direction: step.value.direction }) };
        }
        // A later D-only edit must not carry the last history replay's proof.
        // Selection and document-neutral command bookkeeping leave it intact.
        // PM's docChanged only means the transaction contains a Step.
        if (!hasBaselineStep && transaction.docChanged && !transaction.doc.eq(before.doc)) next = { baseline: previous.baseline,
          revision: previous.revision + 1, origin: transaction.getMeta("input.agent") ? "agent" : "manual" };
        return next;
      },
    },
    props: { handleScrollToSelection: view => reviewReplays.has(view.state) },
    appendTransaction(transactions, before, after) {
      if (!syncBaseline || !transactions.some(transaction => transaction.docChanged)
        || transactions.some(transaction => !shouldSynchronizeReviewBaseline(
          isHistoryTransaction(transaction) ? "undo" : transaction.getMeta("input.agent") ? "agent"
            : transaction.getMeta(NOTE_REVIEW_ORIGIN) ?? "manual", true,
          transaction.steps.some(step => step instanceof NoteReviewBaselineStep)))) return null;
      const current = baselineKey.getState(after);
      if (!current) return null;
      const synchronized = syncBaseline(before, after, current.baseline);
      if (!synchronized || JSON.stringify(synchronized) === JSON.stringify(current.baseline)) return null;
      // PM tags this as appendedTransaction, attaching its inverse to the
      // original input event instead of creating a second baseline undo item.
      return appendNoteReviewBaseline(after, after.tr, synchronized, { origin: "manual", isolate: false });
    },
    view: () => ({ update(view, previousState) {
      const current = baselineKey.getState(view.state);
      if (current && current !== baselineKey.getState(previousState)) onChange?.(current);
    } }),
  });
}

export function readNoteReviewHistory(state: EditorState): NoteReviewHistoryState | undefined {
  return baselineKey.getState(state);
}

/** Append B to the same transaction as D changes. Manual ordinary-region
 * baseline synchronization must share its input transaction; review and Agent
 * operations isolate themselves from adjacent typing. */
export function appendNoteReviewBaseline(state: EditorState, transaction: Transaction, nextBaseline: NoteDocumentV1,
  options: { readonly origin: NoteReviewOrigin; readonly requestId?: string; readonly addToHistory?: boolean; readonly isolate?: boolean }): Transaction {
  const previous = baselineKey.getState(state);
  if (!previous) throw new Error("笔记审阅历史尚未安装");
  const after = snapshot(nextBaseline);
  if (after.pageId !== previous.baseline.pageId) throw new Error("笔记审阅历史不能跨页");
  if (options.requestId !== undefined && !options.requestId.length) throw new Error("无效的笔记审阅请求身份");
  if (options.isolate ?? (options.origin === "review" || options.origin === "agent")) closeHistory(transaction);
  transaction.step(new NoteReviewBaselineStep({ before: previous.baseline, after, origin: options.origin,
    ...(options.origin === "agent" ? { installationId: crypto.randomUUID() } : {}),
    ...(options.requestId === undefined ? {} : { requestId: options.requestId }) }));
  transaction.setMeta(NOTE_REVIEW_ORIGIN, options.origin);
  if (options.origin === "agent") transaction.setMeta("input.agent", true);
  if (options.addToHistory !== undefined) transaction.setMeta("addToHistory", options.addToHistory);
  return transaction;
}
