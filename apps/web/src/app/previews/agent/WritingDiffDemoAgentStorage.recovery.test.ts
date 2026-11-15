import { afterEach, expect, it } from "vitest";
import type { AgentWorkspaceBuffer } from "@research-assistant/core/agent";
import { WritingDiffDemoStorage } from "./WritingDiffDemoStorage";
import { WritingDiffDemoAgentStorage } from "./WritingDiffDemoAgentStorage";
import { initialChanges } from "../../../core/features/agent/WritingDiffFixtures";

const manuscriptId = "00000000-0000-4000-8000-000000000003";
const agents: WritingDiffDemoAgentStorage[] = [];
const observation = ({ path, sessionId, revision, text }: AgentWorkspaceBuffer) => ({ path, sessionId, revision, text });
afterEach(() => { for (const agent of agents.splice(0)) agent.history.dispose(); });
async function fixture() {
  const scope = { manuscriptId, conversationId: crypto.randomUUID(), requestId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID() };
  const storage = new WritingDiffDemoStorage(manuscriptId, initialChanges);
  const agent = new WritingDiffDemoAgentStorage(storage, { admission: { scope, turnId: crypto.randomUUID() }, changes: initialChanges });
  agents.push(agent); await agent.initialized;
  const review = await agent.reviews.review(scope);
  const proposal = await agent.gateways.delivery.proposals.publish(review);
  const applicationScope = { manuscriptId, conversationId: scope.conversationId, proposalId: proposal.proposalId, operationId: crypto.randomUUID() };
  const buffers = await Promise.all(initialChanges.map(async change => ({ path: change.path, sessionId: crypto.randomUUID(), revision: crypto.randomUUID(),
    text: change.before, expectedDiskSha256: (await storage.files.readFile(manuscriptId, change.path)).sha256 })));
  const execute = agent.gateways.application.applications.execute;
  await execute({ action: "begin", scope: applicationScope, buffers });
  const read = () => execute({ action: "read", scope: applicationScope });
  const resume = (live: readonly AgentWorkspaceBuffer[]) => execute({ action: "resume", scope: applicationScope, buffers: live });
  return { agent, storage, review, applicationScope, buffers, execute, read, resume };
}

it("resumes the untouched plan with current process editors and retains frozen history and dispatch CAS", async () => {
  const f = await fixture(), original = await f.read();
  expect(await f.resume(f.buffers)).toEqual(original);
  const current = f.buffers.map(buffer => ({ ...buffer, sessionId: crypto.randomUUID(), revision: crypto.randomUUID() }));
  await expect(f.resume(current)).rejects.toThrow();
  expect(await f.read()).toEqual(original);
  f.agent.recoverOnStartup();
  const interrupted = await f.read();
  await expect(f.resume(current.map((buffer, index) => index === 2 ? { ...buffer, text: "later input" } : buffer))).rejects.toThrow();
  expect(await f.read()).toEqual(interrupted);
  const resumed = await f.resume(current);
  expect(resumed).toMatchObject({ kind: "state", scope: f.applicationScope, status: "applying" });
  if (resumed.kind !== "state") throw new Error("Expected application record");
  expect(resumed.files.map(file => file.target)).toEqual(current.map(buffer => ({ kind: "editor", sessionId: buffer.sessionId,
    revision: buffer.revision, resultRevision: null })));
  expect((await f.agent.gateways.delivery.proposals.review(f.applicationScope)).review).toEqual(f.review);
  const buffer = current[0]!;
  await expect(f.execute({ action: "prepareEditor", scope: f.applicationScope, current: { ...observation(buffer), revision: crypto.randomUUID() } })).rejects.toThrow();
  const dispatch = await f.execute({ action: "prepareEditor", scope: f.applicationScope, current: observation(buffer) });
  expect(dispatch).toMatchObject({ kind: "edit", expected: { sessionId: buffer.sessionId, revision: buffer.revision }, text: initialChanges[0]!.after });
  f.agent.recoverOnStartup();
  const dispatched = await f.read();
  await expect(f.resume(current)).rejects.toThrow();
  expect(await f.read()).toEqual(dispatched);
});

it("does not permit a matching current hash to replace the frozen disk source during recovery", async () => {
  const f = await fixture(); f.agent.recoverOnStartup();
  const previous = await f.read(), buffer = f.buffers[0]!;
  await f.storage.externalEdit(buffer.path, "external actor changed disk");
  const disk = await f.storage.files.readFile(manuscriptId, buffer.path);
  await expect(f.resume(f.buffers.map((entry, index) => index === 0 ? { ...entry, expectedDiskSha256: disk.sha256 } : entry))).rejects.toThrow();
  expect(await f.read()).toEqual(previous);
  expect((await f.storage.files.readFile(manuscriptId, buffer.path)).content).toBe("external actor changed disk");
});

it("keeps every target pending when the injected pre-dispatch write fails", async () => {
  const f = await fixture(); f.agent.failBeforeDispatch = true;
  await expect(f.execute({ action: "prepareEditor", scope: f.applicationScope, current: observation(f.buffers[0]!) })).rejects.toThrow();
  const pending = await f.read();
  if (pending.kind !== "state") throw new Error("Expected application record");
  expect(pending.files.every(file => file.phase === "pending")).toBe(true);
  await expect(f.execute({ action: "deferEditor", scope: f.applicationScope, path: f.buffers[0]!.path })).rejects.toThrow();
  expect(await f.read()).toEqual(pending);
  f.agent.recoverOnStartup(); f.agent.failBeforeDispatch = false;
  const renewed = f.buffers.map(buffer => ({ ...buffer, sessionId: crypto.randomUUID(), revision: crypto.randomUUID() }));
  await f.resume(renewed);
  expect((await f.execute({ action: "prepareEditor", scope: f.applicationScope, current: observation(renewed[0]!) })).kind).toBe("edit");
});
