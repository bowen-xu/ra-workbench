// @vitest-environment happy-dom
import { afterEach, describe, expect, it, vi } from "vitest";
import { BlockNoteEditor } from "@blocknote/core";
import type { AgentNoteApplication, AgentNoteRewriteReview } from "@research-assistant/core/agent";
import { notePageId } from "@research-assistant/core/notes";
import type { NoteDocumentSnapshot, NotesGateway } from "@research-assistant/data/notes";
import { blockNoteToNoteDocument } from "../editor/blockNoteAdapter";
import { createNoteEditorInstallPort } from "../editor/noteEditorInstallPort";
import type { NoteEditorInstance, NoteEditorBlock } from "../editor/noteEditorSchema";
import { NoteSaveController } from "../editor/noteSaveController";
import { NoteAcceptanceController, type NoteAcceptanceSession, type NoteAcceptanceState } from "./noteAcceptanceController";

const scope = { conversationId: "a1111111-1111-4111-8111-111111111111", pageId: "b1111111-1111-4111-8111-111111111111",
  turnId: "c1111111-1111-4111-8111-111111111111", itemId: "d1111111-1111-4111-8111-111111111111" };
const proposalId = `note-rewrite:${"a".repeat(64)}`;
const receipt: AgentNoteApplication = { schemaVersion: 1, ...scope, proposalId, applicationId: scope.itemId,
  beforeRevision: 3, appliedRevision: 4, beforeHash: "a".repeat(64), afterHash: "b".repeat(64), createdAt: "2026-09-10T01:02:03.000Z" };
const cleanup: (() => void)[] = [];
afterEach(() => cleanup.splice(0).forEach(dispose => dispose()));
function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function setup() {
  const engine = BlockNoteEditor.create({ initialContent: [{ id: "first", type: "paragraph", content: "Before" },
    { id: "sibling", type: "paragraph", content: "Keep" }] });
  const element = document.createElement("div"); document.body.append(element); engine.mount(element);
  const read = () => blockNoteToNoteDocument(notePageId(scope.pageId), engine.document as NoteEditorBlock[]);
  const doc = read();
  const baseline: NoteDocumentSnapshot = { document: doc, page: { schemaVersion: 1, id: doc.pageId, title: "Note",
    position: 0, status: "active", isFavorite: false, revision: 3, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now" } };
  const before = doc.blocks[0]?.content;
  if (before?.kind !== "inline") throw new Error("Invalid fixture");
  const after = { kind: "inline" as const, items: [{ type: "text" as const, text: "After", styles: [] }] };
  const review: AgentNoteRewriteReview = { schemaVersion: 1, scope, currentRevision: 3, baselineMatches: true,
    proposal: { schemaVersion: 1, kind: "noteInlineRewrite", status: "proposed", proposalId,
      conversationId: scope.conversationId, pageId: scope.pageId, turnId: scope.turnId, contextSnapshotId: scope.itemId,
      sourceId: `note:${"b".repeat(64)}`, revision: `revision:3:sha256:${"c".repeat(64)}`,
      anchor: JSON.stringify({ schemaVersion: 1, pageId: scope.pageId, pageRevision: 3, blockId: null }),
      edits: [{ blockId: "first", before, after }],
    } };
  const applied: NoteDocumentSnapshot = { ...baseline, page: { ...baseline.page, revision: 4 },
    document: { ...doc, blocks: doc.blocks.map(block => block.id === "first" ? { ...block, content: after } : block) } };
  const saveDocument = vi.fn(async (request: { document: typeof doc; expectedRevision: number }) => ({
    ...baseline, page: { ...baseline.page, revision: request.expectedRevision + 1 }, document: request.document,
  }));
  const undoGateway = {status: vi.fn(async () => null), undo: vi.fn(async () => ({schemaVersion: 1 as const, ...scope, proposalId, applicationId: receipt.applicationId, undoId: scope.turnId, beforeRevision: 4, restoredRevision: 5, beforeHash: receipt.afterHash, restoredHash: receipt.beforeHash, createdAt: receipt.createdAt}))};
  const gateway = { accept: vi.fn(async () => receipt), status: vi.fn(async () => null) };
  const notes = { loadDocument: vi.fn(async () => applied) };
  let session: NoteAcceptanceSession | undefined;
  const saves = new NoteSaveController({ pageId: doc.pageId, initialRevision: 3,
    gateway: { saveDocument } as unknown as NotesGateway,
    onSnapshot: snapshot => { if (session) session = { ...session, snapshot }; }, onStatus: vi.fn(),
  });
  const editor = createNoteEditorInstallPort({ editor: engine as unknown as NoteEditorInstance, pageId: doc.pageId, canInstall: () => true });
  session = { snapshot: baseline, saves, editor };
  const unsubscribe = engine.onChange(() => { if (!editor.isInstalling) saves.enqueue(read()); });
  const states: NoteAcceptanceState[] = [];
  const controller = new NoteAcceptanceController({ gateway, undoGateway, notes, current: () => session, onState: state => states.push(state) });
  cleanup.push(() => { controller.dispose(); saves.dispose(); editor.dispose(); unsubscribe(); engine.unmount(); element.remove(); });
  return { controller, gateway, undoGateway, notes, saves, engine, read, review, baseline, applied, states, saveDocument,
    getSession: () => session, clearSession: () => { session = undefined; },
    replaceSession: (next: NoteAcceptanceSession) => { session = next; } };
}

describe("note acceptance through real save and editor ports", () => {
  it("undoes an applied proposal in the original editor and preserves undo history and the next save revision", async () => {
    const f = setup(); await f.controller.accept(f.review);
    f.notes.loadDocument.mockResolvedValue({...f.baseline, page: {...f.baseline.page, revision: 5}});
    await f.controller.undo(f.review, receipt);
    expect(f.read()).toEqual(f.baseline.document); expect(f.undoGateway.undo).toHaveBeenCalledOnce();
    expect(f.saves.hasExternalChange()).toBe(false);
    expect(f.engine.undo()).toBe(true); expect(f.read()).toEqual(f.applied.document);
    await f.saves.flush(); expect(f.saveDocument).toHaveBeenCalledWith(expect.objectContaining({expectedRevision: 5}));
  });
  it("retains the original undo request after a lost reply and only reloads after a known receipt", async () => {
    const f = setup(); await f.controller.accept(f.review);
    f.undoGateway.undo.mockRejectedValueOnce(new Error("undo reply lost"));
    await expect(f.controller.undo(f.review, receipt)).rejects.toThrow();
    await expect(f.controller.accept(f.review)).rejects.toThrow();
    f.notes.loadDocument.mockRejectedValueOnce(new Error("read failed"));
    await expect(f.controller.retry()).rejects.toThrow();
    expect(f.undoGateway.undo.mock.calls[1]).toEqual(f.undoGateway.undo.mock.calls[0]);
    f.notes.loadDocument.mockResolvedValue({...f.baseline, page: {...f.baseline.page, revision: 5}});
    await f.controller.retry(); expect(f.undoGateway.undo).toHaveBeenCalledTimes(2);
    expect(f.read()).toEqual(f.baseline.document);
  });
  it("does not overwrite new input with an undo result", async () => {
    const f = setup(); await f.controller.accept(f.review);
    const pending = deferred<Awaited<ReturnType<typeof f.undoGateway.undo>>>(); f.undoGateway.undo.mockReturnValueOnce(pending.promise);
    const first = f.controller.undo(f.review, receipt);
    await expect(f.controller.undo(f.review, receipt)).rejects.toThrow();
    f.engine.updateBlock("first", {content: "New input"}); const local = f.read();
    f.notes.loadDocument.mockResolvedValue({...f.baseline, page: {...f.baseline.page, revision: 5}});
    pending.resolve({schemaVersion: 1, ...scope, proposalId, applicationId: receipt.applicationId, undoId: scope.turnId, beforeRevision: 4, restoredRevision: 5, beforeHash: receipt.afterHash, restoredHash: receipt.beforeHash, createdAt: receipt.createdAt});
    await expect(first).rejects.toThrow(); expect(f.read()).toEqual(local);
    expect(f.saves.hasExternalChange()).toBe(true); await f.saves.retry(); expect(f.saveDocument).not.toHaveBeenCalled();
  });
  it("refuses stale and mismatched undo applications before dispatch", async () => {
    const f = setup(); await f.controller.accept(f.review);
    await expect(f.controller.undo(f.review, {...receipt, pageId: scope.itemId})).rejects.toThrow();
    const current = f.getSession()!;
    f.replaceSession({...current, snapshot: {...current.snapshot, page: {...current.snapshot.page, revision: 5}}});
    await expect(f.controller.undo(f.review, receipt)).rejects.toThrow();
    expect(f.undoGateway.undo).not.toHaveBeenCalled();
  });
  it("applies once, keeps sibling/history, and autosaves subsequent undo against the new revision", async () => {
    const f = setup(); await f.controller.accept(f.review);
    expect(f.read()).toEqual(f.applied.document);
    expect(f.getSession()?.snapshot).toEqual(f.applied);
    expect(f.gateway.accept).toHaveBeenCalledOnce(); expect(f.saveDocument).not.toHaveBeenCalled();
    expect(f.saves.hasExternalChange()).toBe(false);
    expect(f.engine.undo()).toBe(true); expect(f.read()).toEqual(f.baseline.document);
    await f.saves.flush();
    expect(f.saveDocument).toHaveBeenCalledWith(expect.objectContaining({ expectedRevision: 4, document: f.baseline.document }));
  });
  it("recovers a lost response using the original request and never runs another autosave", async () => {
    const f = setup(); f.gateway.accept.mockRejectedValueOnce(new Error("reply lost"));
    await expect(f.controller.accept(f.review)).rejects.toThrow("reply lost");
    expect(f.saves.hasExternalChange()).toBe(true); expect(f.states.at(-1)?.phase).toBe("recovery");
    expect(f.gateway.accept).toHaveBeenCalledOnce();
    await f.controller.retry();
    expect(f.gateway.accept.mock.calls[1]).toEqual(f.gateway.accept.mock.calls[0]);
    expect(f.read()).toEqual(f.applied.document); expect(f.saveDocument).not.toHaveBeenCalled();
  });
  it("retains a known receipt while retrying only a failed snapshot read", async () => {
    const f = setup(); f.notes.loadDocument.mockRejectedValueOnce(new Error("read failed"));
    await expect(f.controller.accept(f.review)).rejects.toThrow(); await f.controller.retry();
    expect(f.gateway.accept).toHaveBeenCalledOnce(); expect(f.notes.loadDocument).toHaveBeenCalledTimes(2);
    expect(f.read()).toEqual(f.applied.document);
  });
  it("blocks duplicate acceptance and preserves edits made while native acceptance is pending", async () => {
    const f = setup(), pending = deferred<AgentNoteApplication>(); f.gateway.accept.mockReturnValueOnce(pending.promise);
    const first = f.controller.accept(f.review);
    await expect(f.controller.accept(f.review)).rejects.toThrow();
    f.engine.updateBlock("first", { content: "New user input" });
    const latest = f.read(); pending.resolve(receipt);
    await expect(first).rejects.toThrow();
    expect(f.read()).toEqual(latest); expect(f.saves.hasExternalChange()).toBe(true);
    await f.saves.retry(); expect(f.saveDocument).not.toHaveBeenCalled();
  });
  it("does not install historical receipts over a later stored revision or unrelated content", async () => {
    const f = setup(); f.notes.loadDocument.mockResolvedValueOnce({ ...f.applied, page: { ...f.applied.page, revision: 5 } });
    await expect(f.controller.accept(f.review)).rejects.toThrow(); expect(f.read()).toEqual(f.baseline.document);
    f.notes.loadDocument.mockResolvedValueOnce({ ...f.applied, document: f.baseline.document });
    await expect(f.controller.retry()).rejects.toThrow(); expect(f.read()).toEqual(f.baseline.document);
    expect(f.gateway.accept).toHaveBeenCalledOnce();
  });
  it("ignores a pending result after disposal or switching the active session", async () => {
    const f = setup(), pending = deferred<AgentNoteApplication>(); f.gateway.accept.mockReturnValueOnce(pending.promise);
    const first = f.controller.accept(f.review); f.clearSession(); f.controller.resetSession();
    pending.resolve(receipt); await first;
    expect(f.notes.loadDocument).not.toHaveBeenCalled(); expect(f.read()).toEqual(f.baseline.document);
    expect(f.states.at(-1)?.phase).toBe("idle");
    f.controller.dispose(); await expect(f.controller.accept(f.review)).rejects.toThrow();
  });
  it("rejects dirty, wrong-scope and stale proposals before native dispatch", async () => {
    const f = setup();
    await expect(f.controller.accept({ ...f.review, baselineMatches: false })).rejects.toThrow();
    await expect(f.controller.accept({ ...f.review, scope: { ...scope, pageId: scope.itemId } })).rejects.toThrow();
    f.engine.updateBlock("sibling", { content: "Dirty" });
    await expect(f.controller.accept(f.review)).rejects.toThrow();
    expect(f.gateway.accept).not.toHaveBeenCalled();
  });
  it("allows explicit local conflict resolution with a new save owner, fencing a late acceptance", async () => {
    const f = setup(), pending = deferred<AgentNoteApplication>(); f.gateway.accept.mockReturnValueOnce(pending.promise);
    const first = f.controller.accept(f.review);
    f.engine.updateBlock("first", { content: "Keep my local input" });
    const local = f.read(), original = f.getSession();
    if (!original) throw new Error("Missing session");
    f.saves.dispose();
    const saveDocument = vi.fn(async (request: { document: typeof local; expectedRevision: number }) => ({
      ...f.applied, document: request.document, page: { ...f.applied.page, revision: request.expectedRevision + 1 },
    }));
    const next = new NoteSaveController({ pageId: local.pageId, initialRevision: 4,
      gateway: { saveDocument } as unknown as NotesGateway, onSnapshot: vi.fn(), onStatus: vi.fn() });
    cleanup.push(() => next.dispose());
    f.replaceSession({ ...original, saves: next }); f.controller.resetSession();
    next.restore(local, "dirty"); await next.retry();
    pending.resolve(receipt); await first;
    expect(saveDocument).toHaveBeenCalledWith(expect.objectContaining({ expectedRevision: 4, document: local }));
    expect(f.read()).toEqual(local); expect(f.notes.loadDocument).not.toHaveBeenCalled();
  });
});
