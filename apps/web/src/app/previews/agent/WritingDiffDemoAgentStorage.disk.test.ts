import { afterEach, expect, it, vi } from "vitest";
import { WritingDiffDemoStorage } from "./WritingDiffDemoStorage";
import { WritingDiffDemoAgentStorage } from "./WritingDiffDemoAgentStorage";
import type { DemoChange } from "../../../core/features/agent/WritingDiffFixtures";
import { createWritingProposalApplication } from "../../../../../../plugins/writing_projects/src/features/writing/agent/writingProposalApplication";
import { recoverWritingApplication } from "../../../../../../plugins/writing_projects/src/features/writing/agent/writingApplicationRecovery";
import { createWritingEditorApplication } from "../../../../../../plugins/writing_projects/src/features/writing/editor/editorApplication";

const agents: WritingDiffDemoAgentStorage[] = [];
afterEach(() => { for (const agent of agents.splice(0)) agent.history.dispose(); vi.restoreAllMocks(); });
async function fixture(kind: DemoChange["kind"]) {
  const scope = { manuscriptId: crypto.randomUUID(), conversationId: crypto.randomUUID(),
    requestId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID() };
  const change = { path: "assets/record.json", kind, before: "old record\r\n中", after: "new record\r\n😀", additions: 1, deletions: 1 };
  const storage = new WritingDiffDemoStorage(scope.manuscriptId, [change]);
  const agent = new WritingDiffDemoAgentStorage(storage, { admission: { scope, turnId: crypto.randomUUID() }, changes: [change] });
  agents.push(agent); await agent.initialized;
  const proposal = await agent.gateways.delivery.proposals.publish(await agent.reviews.review(scope));
  const application = { manuscriptId: scope.manuscriptId, conversationId: scope.conversationId,
    proposalId: proposal.proposalId, operationId: crypto.randomUUID() };
  const gateway = agent.gateways.application.applications;
  const editor = createWritingEditorApplication(gateway, () => null);
  const acquire = async () => ({ buffers: [], isCurrent: () => true, refresh: async () => undefined, release: () => undefined });
  const controller = createWritingProposalApplication(gateway, editor, acquire);
  return { scope, change, storage, agent, application, gateway, editor, acquire, controller };
}

it.each(["added", "deleted", "modified"] as const)("applies %s disk targets through the production coordinator without editor buffers or replay", async kind => {
  const f = await fixture(kind);
  const result = await f.controller.apply(f.application);
  expect(result).toMatchObject({ kind: "state", state: { status: "applied", files: [
    { path: f.change.path, phase: "acknowledged", target: { kind: "disk" } },
  ] } });
  const files = await f.storage.files.listFiles(f.scope.manuscriptId);
  expect(files).toHaveLength(kind === "deleted" ? 0 : 1);
  if (kind !== "deleted") {
    expect((await f.storage.files.readFile(f.scope.manuscriptId, f.change.path)).content).toBe(f.change.after);
    await f.storage.externalEdit(f.change.path, "later external input");
  }
  expect(await f.controller.apply({ ...f.application, operationId: crypto.randomUUID() })).toEqual(result);
  if (kind !== "deleted") expect((await f.storage.files.readFile(f.scope.manuscriptId, f.change.path)).content).toBe("later external input");
});

it.each(["added", "deleted"] as const)("reconciles %s after a lost disk confirmation without repeating a write", async kind => {
  const f = await fixture(kind);
  f.agent.failDiskConfirmation = true;
  expect(await f.controller.apply(f.application)).toMatchObject({ kind: "unconfirmed", state: {
    status: "recoveryRequired", files: [{ phase: "dispatched" }],
  } });
  f.agent.recoverOnStartup(); f.agent.failDiskConfirmation = false;
  expect(await recoverWritingApplication(f.application, f.change.path, f.gateway, f.editor, f.acquire))
    .toMatchObject({ kind: "state", state: { status: "applied", files: [{ phase: "acknowledged" }] } });
  expect((await f.storage.files.listFiles(f.scope.manuscriptId)).length).toBe(kind === "deleted" ? 0 : 1);
});

it.each(["added", "deleted", "modified"] as const)("reconciles %s when the file write succeeded but its reply was lost", async kind => {
  const f = await fixture(kind);
  f.storage.fault = "fileReceipt";
  expect(await f.controller.apply(f.application)).toMatchObject({ kind: "unconfirmed", state: {
    status: "recoveryRequired", files: [{ phase: "dispatched" }],
  } });
  const writes = [] as string[];
  const apply = f.storage.files.applyChange;
  f.storage.files.applyChange = async (...args) => { writes.push(args[1]); return apply(...args); };
  f.agent.recoverOnStartup();
  expect(await recoverWritingApplication(f.application, f.change.path, f.gateway, f.editor, f.acquire))
    .toMatchObject({ kind: "state", state: { status: "applied", files: [{ phase: "acknowledged" }] } });
  expect(writes).toEqual([]);
});

it.each(["added", "deleted", "modified"] as const)("preserves external content instead of guessing that %s completed", async kind => {
  const f = await fixture(kind);
  f.agent.failDiskConfirmation = true;
  await f.controller.apply(f.application);
  if (kind === "deleted") await f.storage.files.createFile(f.scope.manuscriptId, f.change.path, "external replacement");
  else await f.storage.externalEdit(f.change.path, "external replacement");
  f.agent.failDiskConfirmation = false;
  expect(await recoverWritingApplication(f.application, f.change.path, f.gateway, f.editor, f.acquire))
    .toMatchObject({ kind: "state", state: { status: "recoveryRequired", files: [{ phase: "dispatched" }] } });
  expect((await f.storage.files.readFile(f.scope.manuscriptId, f.change.path)).content).toBe("external replacement");
});

it("rejects an added file collision before creating an application journal", async () => {
  const f = await fixture("added");
  await f.storage.files.createFile(f.scope.manuscriptId, f.change.path, "external replacement");
  expect(await f.controller.apply(f.application)).toMatchObject({ kind: "unconfirmed", state: null });
  expect(await f.gateway.find(f.application)).toBeNull();
  expect((await f.storage.files.readFile(f.scope.manuscriptId, f.change.path)).content).toBe("external replacement");
});

it("rejects removal of a file with a live editor buffer", async () => {
  const f = await fixture("deleted");
  const before = await f.storage.files.readFile(f.scope.manuscriptId, f.change.path);
  const acquire = async () => ({ ...(await f.acquire()), buffers: [{ path: f.change.path, sessionId: crypto.randomUUID(),
    revision: crypto.randomUUID(), expectedDiskSha256: before.sha256, text: before.content }] });
  const controller = createWritingProposalApplication(f.gateway, f.editor, acquire);
  expect(await controller.apply(f.application)).toMatchObject({ kind: "unconfirmed", state: null });
  expect(await f.gateway.find(f.application)).toBeNull();
  expect((await f.storage.files.readFile(f.scope.manuscriptId, f.change.path)).content).toBe(before.content);
});

it.each(["deleted", "modified"] as const)("requires the original backup when reconciling a dispatched %s target", async kind => {
  const f = await fixture(kind);
  f.agent.failDiskConfirmation = true;
  await f.controller.apply(f.application);
  f.agent.failDiskConfirmation = false;
  vi.spyOn(f.storage.files, "readPreserved").mockResolvedValue(null);
  expect(await recoverWritingApplication(f.application, f.change.path, f.gateway, f.editor, f.acquire))
    .toMatchObject({ kind: "state", state: { status: "recoveryRequired", files: [{ phase: "dispatched" }] } });
});
