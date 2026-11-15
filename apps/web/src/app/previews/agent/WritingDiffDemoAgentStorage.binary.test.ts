import { createHash } from "node:crypto";
import { afterEach, expect, it, vi } from "vitest";
import { WritingDiffDemoStorage } from "./WritingDiffDemoStorage";
import { WritingDiffDemoAgentStorage } from "./WritingDiffDemoAgentStorage";
import { binaryFileChanges, type DemoChange } from "../../../core/features/agent/WritingDiffFixtures";
import { createWritingProposalApplication } from "../../../../../../plugins/writing_projects/src/features/writing/agent/writingProposalApplication";
import { recoverWritingApplication } from "../../../../../../plugins/writing_projects/src/features/writing/agent/writingApplicationRecovery";
import { createWritingEditorApplication } from "../../../../../../plugins/writing_projects/src/features/writing/editor/editorApplication";

const agents: WritingDiffDemoAgentStorage[] = [];
afterEach(() => { for (const agent of agents.splice(0)) agent.history.dispose(); vi.restoreAllMocks(); });
async function fixture(changes: readonly DemoChange[]) {
  const scope = { manuscriptId: crypto.randomUUID(), conversationId: crypto.randomUUID(), requestId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID() };
  const storage = new WritingDiffDemoStorage(scope.manuscriptId, changes);
  const agent = new WritingDiffDemoAgentStorage(storage, { admission: { scope, turnId: crypto.randomUUID() }, changes });
  agents.push(agent); await agent.initialized;
  const review = await agent.reviews.review(scope);
  const proposal = await agent.gateways.delivery.proposals.publish(review);
  const application = { manuscriptId: scope.manuscriptId, conversationId: scope.conversationId, proposalId: proposal.proposalId, operationId: crypto.randomUUID() };
  const gateway = agent.gateways.application.applications, editor = createWritingEditorApplication(gateway, () => null);
  const acquire = async () => ({ buffers: [], isCurrent: () => true, refresh: async () => undefined, release: () => undefined });
  const controller = createWritingProposalApplication(gateway, editor, acquire);
  return { scope, storage, agent, review, application, gateway, editor, acquire, controller };
}

it.each(["modified", "added", "deleted"] as const)("preserves exact %s binary bytes and frozen metadata through production application", async kind => {
  const change = { ...binaryFileChanges[1]!, kind }, f = await fixture([change]);
  expect(f.storage.initialOpenPaths).toEqual([]);
  const historical = await f.agent.gateways.delivery.proposals.readFile({ proposal: f.application, path: change.path });
  expect(historical.file.before).toEqual(kind === "added" ? null : { kind: "binary" });
  expect(historical.file.after).toEqual(kind === "deleted" ? null : { kind: "binary" });
  expect(await f.controller.apply(f.application)).toMatchObject({ kind: "state", state: { status: "applied" } });
  if (kind === "deleted") await expect(f.storage.files.readBytes(f.scope.manuscriptId, change.path)).rejects.toThrow();
  else {
    const bytes = await f.storage.files.readBytes(f.scope.manuscriptId, change.path);
    expect([...bytes]).toEqual(change.afterBytes);
    const metadata = (await f.storage.files.listFiles(f.scope.manuscriptId))[0]!;
    expect(metadata).toMatchObject({ sizeBytes: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex") });
    await expect(f.storage.files.readFile(f.scope.manuscriptId, change.path)).rejects.toThrow("二进制");
    bytes[0] = 42;
    expect([...(await f.storage.files.readBytes(f.scope.manuscriptId, change.path))]).toEqual(change.afterBytes);
  }
  if (kind !== "added") expect([...(await f.storage.files.readPreserved(f.scope.manuscriptId, change.path, f.application.operationId))!]).toEqual(change.beforeBytes);
  expect((await f.agent.gateways.delivery.proposals.readFile({ proposal: f.application, path: change.path })).file).toEqual(historical.file);
});

it.each(["fileReceipt", "diskConfirm"] as const)("reconciles binary %s failure using exact bytes and backup without another write", async fault => {
  const change = binaryFileChanges[1]!, f = await fixture([change]);
  if (fault === "fileReceipt") f.storage.fault = fault;
  else f.agent.failDiskConfirmation = true;
  expect(await f.controller.apply(f.application)).toMatchObject({ kind: "unconfirmed", state: { status: "recoveryRequired", files: [{ phase: "dispatched" }] } });
  f.agent.recoverOnStartup(); f.storage.fault = "none"; f.agent.failDiskConfirmation = false;
  const writes = vi.spyOn(f.storage.files, "applyChange");
  expect(await recoverWritingApplication(f.application, change.path, f.gateway, f.editor, f.acquire)).toMatchObject({ kind: "state", state: { status: "applied" } });
  expect(writes).not.toHaveBeenCalled();
  expect([...(await f.storage.files.readBytes(f.scope.manuscriptId, change.path))]).toEqual(change.afterBytes);
});

it("keeps unconfirmed binary delivery uncertain when preserved originals are missing", async () => {
  const change = binaryFileChanges[1]!, f = await fixture([change]);
  f.agent.failDiskConfirmation = true; await f.controller.apply(f.application); f.agent.failDiskConfirmation = false;
  vi.spyOn(f.storage.files, "readPreserved").mockResolvedValue(null);
  expect(await recoverWritingApplication(f.application, change.path, f.gateway, f.editor, f.acquire)).toMatchObject({ kind: "state", state: { status: "recoveryRequired" } });
});

it("rejects a text-to-binary change in a live buffer before writing any other disk target", async () => {
  const first: DemoChange = { path: "added.txt", kind: "added", before: "", after: "new", additions: 1, deletions: 0 };
  const changed: DemoChange = { path: "main.tex", kind: "modified", before: "source", after: "",
    afterBytes: binaryFileChanges[1]!.afterBytes!, additions: 0, deletions: 1 };
  const f = await fixture([first, changed]), disk = await f.storage.files.readFile(f.scope.manuscriptId, changed.path);
  const acquire = async () => ({ ...(await f.acquire()), buffers: [{ path: changed.path, sessionId: crypto.randomUUID(), revision: crypto.randomUUID(), expectedDiskSha256: disk.sha256, text: disk.content }] });
  expect(await createWritingProposalApplication(f.gateway, f.editor, acquire).apply(f.application)).toMatchObject({ kind: "unconfirmed", state: null });
  expect(await f.gateway.find(f.application)).toBeNull();
  expect((await f.storage.files.listFiles(f.scope.manuscriptId)).map(file => file.relativePath)).toEqual([changed.path]);
  expect((await f.storage.files.readFile(f.scope.manuscriptId, changed.path)).content).toBe("source");
});

it("classifies invalid UTF-8 and oversized history without discarding stored bytes", async () => {
  const invalid: DemoChange = { ...binaryFileChanges[1]!, beforeBytes: [255], afterBytes: [128, 129] };
  const large: DemoChange = { path: "large.txt", kind: "added", before: "", after: "x".repeat(1024 * 1024 + 1), additions: 1, deletions: 0 };
  const f = await fixture([invalid, large]);
  const read = (path: string) => f.agent.gateways.delivery.proposals.readFile({ proposal: f.application, path });
  expect((await read(invalid.path)).file.after).toEqual({ kind: "binary" });
  expect((await read(large.path)).file.after).toEqual({ kind: "tooLarge" });
  expect(await f.controller.apply(f.application)).toMatchObject({ kind: "state", state: { status: "applied" } });
  expect((await f.storage.files.readBytes(f.scope.manuscriptId, large.path)).length).toBe(1024 * 1024 + 1);
});

it.each([1024 * 1024 + 1, 4 * 1024 * 1024 + 1])("uses the native source limit independently of bounded history for %i bytes", async size => {
  const change: DemoChange = { path: "main.tex", kind: "modified", before: "source", after: "x".repeat(size), additions: 1, deletions: 1 };
  const f = await fixture([change]), source = await f.storage.files.readFile(f.scope.manuscriptId, change.path);
  const current = { path: change.path, sessionId: crypto.randomUUID(), revision: crypto.randomUUID(), text: source.content };
  expect((await f.agent.gateways.delivery.proposals.readFile({ proposal: f.application, path: change.path })).file.after).toEqual({ kind: "tooLarge" });
  const begin = f.gateway.execute({ scope: f.application, action: "begin", buffers: [{ ...current, expectedDiskSha256: source.sha256 }] });
  if (size > 4 * 1024 * 1024) {
    await expect(begin).rejects.toThrow(); expect(await f.gateway.find(f.application)).toBeNull();
  } else {
    await expect(begin).resolves.toMatchObject({ kind: "state", files: [{ target: { kind: "editor" } }] });
    await expect(f.gateway.execute({ scope: f.application, action: "prepareEditor", current })).resolves.toMatchObject({ kind: "edit", text: change.after });
  }
  expect((await f.storage.files.readFile(f.scope.manuscriptId, change.path)).content).toBe("source");
});
