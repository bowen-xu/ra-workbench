import { afterEach, expect, it, vi } from "vitest";
import { WritingDiffDemoStorage, writingDiffDemoHash } from "./WritingDiffDemoStorage";
import { WritingDiffDemoAgentStorage } from "./WritingDiffDemoAgentStorage";
import { mixedFileChanges, binaryFileChanges, type DemoChange } from "../../../core/features/agent/WritingDiffFixtures";
import { writingDiffDemoBytes } from "./WritingDiffDemoBytes";
import { WritingFileSessions } from "../../../../../../plugins/writing_projects/src/features/writing/editor/writingFileSessions";
import { createWritingEditorApplication } from "../../../../../../plugins/writing_projects/src/features/writing/editor/editorApplication";
import { createWritingEditorUndo } from "../../../../../../plugins/writing_projects/src/features/writing/editor/editorUndo";
import { createWritingEditorRestoration } from "../../../../../../plugins/writing_projects/src/features/writing/editor/editorRestoration";
import { createWritingProposalApplication } from "../../../../../../plugins/writing_projects/src/features/writing/agent/writingProposalApplication";
import { createWritingProposalUndo } from "../../../../../../plugins/writing_projects/src/features/writing/agent/writingProposalUndo";
import { rollbackWritingUndo } from "../../../../../../plugins/writing_projects/src/features/writing/agent/writingUndoRollback";
import type { AgentApplicationScope } from "@research-assistant/core/agent";
import { parseManuscriptPath } from "@research-assistant/writing";

const cleanups: (() => void)[] = [];
afterEach(() => { for (const cleanup of cleanups.splice(0)) cleanup(); vi.restoreAllMocks(); });
async function fixture(changes: readonly DemoChange[] = mixedFileChanges) {
  const scope = { manuscriptId: crypto.randomUUID(), conversationId: crypto.randomUUID(), requestId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID() };
  const storage = new WritingDiffDemoStorage(scope.manuscriptId, changes);
  const agent = new WritingDiffDemoAgentStorage(storage, { admission: { scope, turnId: crypto.randomUUID() }, changes });
  cleanups.push(() => agent.history.dispose());
  await agent.initialized;
  const review = await agent.reviews.review(scope), proposal = await agent.gateways.delivery.proposals.publish(review);
  const original = { manuscriptId: scope.manuscriptId, conversationId: scope.conversationId, proposalId: proposal.proposalId, operationId: crypto.randomUUID() };
  const files = new WritingFileSessions();
  const main = files.open({ manuscriptId: scope.manuscriptId, relativePath: "main.tex" }, changes[0]!.before, await writingDiffDemoHash(changes[0]!.before));
  const readBuffers = () => files.readBuffers(scope.manuscriptId);
  const acquire = vi.fn(async () => ({ buffers: readBuffers(), readBuffers, isCurrent: () => true, refresh: async () => {}, release: () => {} }));
  const resolve = (identity: AgentApplicationScope, path: string) => files.get({ manuscriptId: identity.manuscriptId, relativePath: path })?.applicationPort ?? null;
  const gateways = agent.gateways.application;
  const apply = createWritingProposalApplication(gateways.applications, createWritingEditorApplication(gateways.applications, resolve), acquire);
  expect(await apply.apply(original)).toMatchObject({ kind: "state", state: { status: "applied" } });
  // Formal saving is host-owned; this fixture uses exact mock file I/O after the
  // production coordinator completed, before preparing the independent Undo.
  await storage.files.saveFile({ manuscriptId: scope.manuscriptId, relativePath: parseManuscriptPath("main.tex"), content: main.read().content, expectedSha256: main.savedSha256! });
  main.recordSaved(await writingDiffDemoHash(main.read().content));
  const undoId = crypto.randomUUID();
  const undo = createWritingProposalUndo(gateways.undos, (scope, id) => createWritingEditorUndo(gateways.undos, scope, id, resolve), acquire);
  const restoration = createWritingEditorRestoration(gateways.undoRestorations(original), resolve);
  const rollback = () => rollbackWritingUndo(original, undoId, gateways.undos, acquire, restoration);
  return { storage, agent, main, original, undoId, undo, rollback, acquire, gateways, readBuffers };
}
async function interrupt(changes?: readonly DemoChange[]) {
  const f = await fixture(changes);
  f.storage.fault = "fileReceipt";
  expect(await f.undo.apply(f.original, f.undoId)).toMatchObject({ kind: "unconfirmed", state: { status: "recoveryRequired" } });
  expect(f.main.read().content).toBe(mixedFileChanges[0]!.before);
  f.storage.fault = "none";
  return f;
}
it.each([{ changes: mixedFileChanges }, { changes: binaryFileChanges }])("compensates interrupted independent Undo through production coordinators (%#)", async ({ changes }) => {
  const f = await interrupt(changes);
  expect(await f.rollback()).toMatchObject({ kind: "state", state: { status: "rolledBack" } });
  expect(f.main.read().content).toBe(changes[0]!.after);
  for (const change of changes.slice(1)) {
    if (change.kind === "deleted") expect((await f.storage.files.listFiles(f.original.manuscriptId)).some(file => file.relativePath === change.path)).toBe(false);
    else expect(await f.storage.files.readBytes(f.original.manuscriptId, change.path)).toEqual(writingDiffDemoBytes(change, "after"));
  }
  expect(await f.gateways.applications.execute({ action: "read", scope: f.original })).toMatchObject({ status: "applied" });
  const restore = vi.spyOn(f.storage.files, "restoreChange"), editor = vi.spyOn(f.main.applicationPort, "apply");
  expect(await f.rollback()).toMatchObject({ kind: "state", state: { status: "rolledBack" } });
  expect(restore).not.toHaveBeenCalled(); expect(editor).not.toHaveBeenCalled();
});
it("checks later disk edits before restoring any editor", async () => {
  const f = await interrupt();
  await f.storage.files.createFile(f.original.manuscriptId, parseManuscriptPath("assets/old.json"), "external input");
  const editor = vi.spyOn(f.main.applicationPort, "apply");
  expect(await f.rollback()).toMatchObject({ kind: "unconfirmed" });
  expect(editor).not.toHaveBeenCalled(); expect(f.main.read().content).toBe(mixedFileChanges[0]!.before);
  expect((await f.storage.files.readFile(f.original.manuscriptId, "assets/old.json")).content).toBe("external input");
});
it("recovers a lost disk compensation reply without repeating the editor transaction", async () => {
  const f = await interrupt(); f.storage.fault = "fileReceipt";
  expect(await f.rollback()).toMatchObject({ kind: "unconfirmed" });
  expect(f.main.read().content).toBe(mixedFileChanges[0]!.after);
  const editor = vi.spyOn(f.main.applicationPort, "apply"); f.storage.fault = "none";
  expect(await f.rollback()).toMatchObject({ kind: "state", state: { status: "rolledBack" } });
  expect(editor).not.toHaveBeenCalled();
});
it("keeps later input and refuses forward Undo once compensation began", async () => {
  const f = await interrupt(); f.storage.fault = "fileReceipt";
  expect(await f.rollback()).toMatchObject({ kind: "unconfirmed" });
  f.storage.fault = "none";
  f.main.dispatch({ changes: { from: f.main.state.doc.length, insert: "later input" } });
  expect(await f.rollback()).toMatchObject({ kind: "unconfirmed" });
  expect(f.main.read().content).toBe(mixedFileChanges[0]!.after + "later input");
  await expect(f.gateways.undos.execute({ scope: f.original, undoId: f.undoId, command: { action: "resume", buffers: [] } })).rejects.toThrow();
});
it("confirms the original compensation receipt but stops remaining work after later input", async () => {
  const f = await interrupt(); f.agent.undoStorage.restorationStorage.failConfirmation = true;
  expect(await f.rollback()).toMatchObject({ kind: "unconfirmed" });
  const editor = vi.spyOn(f.main.applicationPort, "apply");
  f.main.dispatch({ changes: { from: f.main.state.doc.length, insert: "later input" } });
  f.agent.undoStorage.restorationStorage.failConfirmation = false;
  expect(await f.rollback()).toMatchObject({ kind: "unconfirmed", state: { files: [expect.objectContaining({ path: "main.tex", phase: "restored" }), expect.anything(), expect.anything()] } });
  expect(editor).not.toHaveBeenCalled();
  expect(f.main.read().content).toBe(mixedFileChanges[0]!.after + "later input");
});
it("cancels an untouched Undo without changing the original applied operation", async () => {
  const f = await fixture();
  const identity = { scope: f.original, undoId: f.undoId };
  await f.gateways.undos.execute({ ...identity, command: { action: "prepare", buffers: f.readBuffers() } });
  expect(await f.gateways.undos.execute({ ...identity, command: { action: "cancelPending" } })).toMatchObject({ result: { status: "rolledBack" } });
  expect(await f.undo.apply(f.original, f.undoId)).toMatchObject({ kind: "state", state: { status: "rolledBack" } });
  expect(f.main.read().content).toBe(mixedFileChanges[0]!.after);
  expect(await f.gateways.applications.execute({ action: "read", scope: f.original })).toMatchObject({ status: "applied" });
});
it("refuses cancellation after any Undo effect was dispatched", async () => {
  const f = await interrupt();
  await expect(f.gateways.undos.execute({ scope: f.original, undoId: f.undoId, command: { action: "cancelPending" } })).rejects.toThrow();
  expect(f.main.read().content).toBe(mixedFileChanges[0]!.before);
});
it("does not treat compensation as permission to redo a completed Undo", async () => {
  const f = await fixture();
  expect(await f.undo.apply(f.original, f.undoId)).toMatchObject({ kind: "state", state: { status: "applied" } });
  expect(await f.rollback()).toMatchObject({ kind: "unavailable" });
  expect(f.main.read().content).toBe(mixedFileChanges[0]!.before);
});
it("rejects an Undo restoration bound to a different original operation", async () => {
  const f = await interrupt();
  const gateway = f.gateways.undoRestorations({ ...f.original, operationId: crypto.randomUUID() });
  await expect(gateway.execute({ action: "read", scope: { ...f.original, operationId: f.undoId }, path: "main.tex" })).rejects.toThrow();
});
it("compensates a partial Undo after the editor result was formally saved", async () => {
  const f = await interrupt();
  const result = await f.storage.files.saveFile({ manuscriptId: f.original.manuscriptId, relativePath: parseManuscriptPath("main.tex"),
    expectedSha256: f.main.savedSha256!, content: f.main.read().content });
  f.main.recordSaved(result.sha256);
  expect(await f.rollback()).toMatchObject({ kind: "state", state: { status: "rolledBack" } });
  expect(f.main.read().content).toBe(mixedFileChanges[0]!.after);
});
