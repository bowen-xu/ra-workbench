import { decodeAgentNoteApplication, decodeAgentNoteRewriteReview, type AgentNoteApplication, type AgentNoteUndo, type AgentNoteRewriteReview } from "@research-assistant/core/agent";
import type { NoteBlockV1, NoteDocumentV1 } from "@research-assistant/core/notes";
import type { AgentNoteApplicationGateway, AgentNoteUndoGateway } from "@research-assistant/data/agent";
import type { NoteDocumentSnapshot, NotesGateway } from "@research-assistant/data/notes";
import type { NoteEditorInstallPort } from "../editor/noteEditorInstallPort";
import type { NoteSaveCoordinator } from "../editor/noteSaveController";
import type { NoteExternalSaveLease } from "../editor/noteExternalSaveLease";

export interface NoteAcceptanceSession {
  readonly snapshot: NoteDocumentSnapshot;
  readonly saves: NoteSaveCoordinator;
  readonly editor: NoteEditorInstallPort;
}
export type NoteAcceptanceState = { readonly phase: "idle" } | {
  readonly phase: "applying" | "recovery";
  readonly pageId: string;
  readonly message: string;
};
interface Operation {
  readonly application?: AgentNoteApplication;
  readonly session: NoteAcceptanceSession;
  readonly review: AgentNoteRewriteReview;
  readonly requestId: string;
  readonly candidate: NoteDocumentV1;
  readonly install: () => undefined;
  readonly lease: NoteExternalSaveLease;
  receipt?: AgentNoteApplication | AgentNoteUndo;
}
function same(left: unknown, right: unknown): boolean {
  const json = (value: unknown) => JSON.stringify(value, (_key, item: unknown) => item && typeof item === "object" && !Array.isArray(item)
    ? Object.fromEntries(Object.entries(item).sort(([a], [b]) => a.localeCompare(b))) : item);
  return json(left) === json(right);
}
function candidateDocument(baseline: NoteDocumentV1, review: AgentNoteRewriteReview, undo: boolean): NoteDocumentV1 {
  const remaining = new Map(review.proposal.edits.map(edit => [edit.blockId, edit]));
  const visit = (blocks: readonly NoteBlockV1[]): NoteBlockV1[] => blocks.map(block => {
    const edit = remaining.get(block.id);
    if (edit && !same(block.content, undo ? edit.after : edit.before)) throw new Error("提案原文与当前笔记不一致，请重新核验。");
    remaining.delete(block.id);
    return { ...block, content: edit ? (undo ? edit.before : edit.after) : block.content, children: visit(block.children) };
  });
  const candidate = { ...baseline, blocks: visit(baseline.blocks) };
  if (remaining.size) throw new Error("提案中的块已不存在，请重新核验。");
  return candidate;
}

/** Workspace-owned operation: hiding a proposal does not lose its pending native
 * request. A new editor/save session can never receive an old operation's result. */
export class NoteAcceptanceController {
  private operation: Operation | undefined;
  private busy = false;
  private disposed = false;
  constructor(private readonly options: {
    readonly gateway: AgentNoteApplicationGateway;
    readonly undoGateway: AgentNoteUndoGateway;
    readonly notes: Pick<NotesGateway, "loadDocument">;
    readonly current: () => NoteAcceptanceSession | undefined;
    readonly onState: (state: NoteAcceptanceState) => void;
    readonly onApplied?: (kind: "accept" | "undo") => void;
  }) {}
  private current(operation: Operation): boolean {
    const session = this.options.current();
    return !this.disposed && this.operation === operation && session?.saves === operation.session.saves
      && session.editor === operation.session.editor && session.snapshot.page.id === operation.review.scope.pageId;
  }
  dispose(): void { this.disposed = true; this.operation = undefined; }
  /** Called when the document view changes. Does not cancel a dispatched native transaction. */
  resetSession(): void { this.operation = undefined; this.busy = false; this.options.onState({ phase: "idle" }); }

  async accept(input: AgentNoteRewriteReview): Promise<void> {
    await this.begin(input);
  }

  async undo(input: AgentNoteRewriteReview, original: AgentNoteApplication): Promise<void> {
    const application = decodeAgentNoteApplication(original, input.scope, input.proposal.proposalId);
    await this.begin(input, application);
  }

  private async begin(input: AgentNoteRewriteReview, application?: AgentNoteApplication): Promise<void> {
    if (this.disposed || this.busy || this.operation) throw new Error("请先核对上一条修改操作的结果。");
    const review = decodeAgentNoteRewriteReview(input, input.scope);
    const session = this.options.current();
    if (!session || session.snapshot.page.status !== "active" || session.snapshot.page.id !== review.scope.pageId
      || (!application && !review.baselineMatches)
      || session.snapshot.page.revision !== (application?.appliedRevision ?? review.currentRevision)) throw new Error("笔记版本已变化，请重新核验。");
    const baseline = structuredClone(session.snapshot.document);
    const candidate = candidateDocument(baseline, review, !!application);
    const install = session.editor.prepare(baseline, candidate);
    const requestId = crypto.randomUUID();
    const lease = session.saves.beginExternalChange(application?.appliedRevision ?? review.currentRevision, baseline);
    if (!lease) throw new Error("请先处理未保存内容，再重新核验提案。");
    const operation: Operation = { session, review, requestId, candidate, install, lease, ...(application ? {application} : {}) };
    this.operation = operation;
    if (!lease.markDispatched()) {
      lease.cancelBeforeDispatch(); this.operation = undefined;
      throw new Error("笔记状态已变化，修改操作未发送。");
    }
    await this.attempt(operation);
  }

  async retry(): Promise<void> {
    if (this.busy || !this.operation || !this.current(this.operation)) return;
    await this.attempt(this.operation);
  }

  private async attempt(operation: Operation): Promise<void> {
    this.busy = true;
    const { review, lease, application } = operation;
    this.options.onState({ phase: "applying", pageId: review.scope.pageId, message: application ? "正在撤销改写并核对已保存版本…" : "正在应用改写并核对已保存版本…" });
    try {
      if (!operation.receipt) {
        operation.receipt = application
          ? await this.options.undoGateway.undo(application, operation.requestId)
          : await this.options.gateway.accept(review.scope, operation.requestId, review.proposal.proposalId, review.currentRevision);
      }
      if (!this.current(operation)) return;
      const saved = await this.options.notes.loadDocument(operation.session.snapshot.page.id);
      if (!this.current(operation)) return;
      const revision = "restoredRevision" in operation.receipt ? operation.receipt.restoredRevision : operation.receipt.appliedRevision;
      if (saved.page.id !== review.scope.pageId || saved.page.revision !== revision
        || saved.document.pageId !== review.scope.pageId || !same(saved.document, operation.candidate)) {
        throw new Error("已保存正文与原操作版本不同；请比较版本，当前编辑内容已保留。");
      }
      await operation.session.saves.synchronizeExternal?.(saved);
      if (!lease.commit(saved, operation.session.saves.synchronizeExternal ? () => undefined : operation.install)) throw new Error("修改期间出现了新编辑，请比较版本；当前输入已保留。");
      this.operation = undefined;
      this.options.onState({ phase: "idle" });
      this.options.onApplied?.(application ? "undo" : "accept");
    } catch (reason: unknown) {
      if (!this.current(operation)) return;
      lease.requireRecovery();
      this.options.onState({ phase: "recovery", pageId: review.scope.pageId,
        message: application ? (operation.receipt ? "撤销已保存，但编辑器尚未同步。可重试核对或比较版本；不会覆盖新输入。" : "撤销结果尚未确认。可重试原撤销操作；不会重复保存。") : operation.receipt ? "改写已保存，但编辑器尚未完成同步。可重试核对或比较版本；不会覆盖新输入。"
          : "接受结果尚未确认。可重试原接受操作；不会创建另一条修改记录。",
      });
      throw reason;
    } finally { if (this.operation === operation || !this.operation) this.busy = false; }
  }
}
