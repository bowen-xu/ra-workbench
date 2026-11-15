import {afterEach, expect, it, vi} from "vitest";
import {agentDraftInput, insertAgentReference, decodeAgentWritingQueuePrepareStatusResponse, type AgentWritingQueuePreviewRequest} from "@research-assistant/core/agent";
import {WritingDiffDemoStorage} from "./WritingDiffDemoStorage";
import {WritingDiffDemoAgentStorage} from "./WritingDiffDemoAgentStorage";
import {createWritingDiffDemoRuntime} from "./writingDiffDemoRuntime";
const cleanups: (() => void)[] = [];
afterEach(() => {cleanups.splice(0).forEach(cleanup => cleanup()); vi.restoreAllMocks();});
async function fixture(text = "read") {
  const manuscriptId = crypto.randomUUID(), conversationId = crypto.randomUUID(), inputId = crypto.randomUUID();
  const files = new WritingDiffDemoStorage(manuscriptId, [{path: "main.tex", before: "original", after: "original", kind: "modified", additions: 0, deletions: 0}]);
  const agent = new WritingDiffDemoAgentStorage(files, undefined), runtime = createWritingDiffDemoRuntime(files, agent, manuscriptId, conversationId);
  cleanups.push(() => runtime.dispose()); const file = await files.files.readFile(manuscriptId, "main.tex");
  const scope = {manuscriptId, conversationId};
  const draft = text === "/compact" ? {text, bindings: []} : insertAgentReference({text, bindings: []}, {type: "skill", name: runtime.execution.resources.name, identity: {
    environment_id: "application", source_id: runtime.execution.resources.sourceId, revision: "1"}});
  const added = await runtime.queue.gateway.edit({...scope, requestId: crypto.randomUUID(), expectedRevision: 0,
    edit: {type: "add", inputId, text: draft.text, userInput: agentDraftInput(draft)}});
  const request: AgentWritingQueuePreviewRequest = {inputId, expectedRevision: added.revision, submission: {conversationId, requestId: crypto.randomUUID(), text: draft.text,
    userInput: agentDraftInput(draft), context: {manuscriptId, files: [{relativePath: "main.tex", expectedDiskSha256: file.sha256}]}, resources: [{
      kind: "manuscript", resourceId: manuscriptId, parts: [], groupAccess: "readOnly", newMemberAccess: "readOnly"}]}};
  return {runtime, files, agent, scope, request, inputId};
}
it("persists complete preparation identity and immutable historical ACK across reopen and unavailable sources", async () => {
  const f = await fixture(); f.runtime.queue.failNextPrepare("afterCommit");
  await expect(f.runtime.queue.preview.prepare(f.request)).rejects.toThrow("ACK_LOST");
  const status = decodeAgentWritingQueuePrepareStatusResponse(await f.runtime.queue.preview.prepareStatus(f.request));
  if (status.status.type !== "committed") throw new Error("fixture proof missing");
  expect(status.status.result.preview.input.userInput).toEqual(f.request.submission.userInput);
  expect(status.status.result.preview.input.skillInstructions?.[0]?.contents).toContain("scripts/run.sh");
  const q = await f.runtime.queue.gateway.read(f.scope); await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: q.revision,
    edit: {type: "add", inputId: crypto.randomUUID(), text: "tail"}});
  f.runtime.queue.restore(f.runtime.queue.snapshot());
  const read = vi.spyOn(f.files.files, "readFile").mockRejectedValue(new Error("source unavailable"));
  expect(await f.runtime.queue.preview.prepare(f.request)).toEqual(status.status.result); expect(read).not.toHaveBeenCalled();
  expect(await f.runtime.queue.preview.prepareStatus(f.request)).toEqual(status);
  const current = await f.runtime.queue.preview.recover(f.scope);
  expect(current?.queue.paused).toBe(true); expect(current!.queue.revision).toBeGreaterThan(status.status.result.queue.revision);
  expect(current?.queue.inputs).toHaveLength(2);
  for (const submission of [{...f.request.submission, resources: []}, {...f.request.submission, retrieval: {traceId: "trace", query: "different"}},
    {...f.request.submission, turnConfig: {maxBytes: 8}}]) await expect(f.runtime.queue.preview.prepare({...f.request, submission})).rejects.toMatchObject({code: "storage", reason: {code: "DATA_SQL_DOMAIN"}});
});
it("readonly NotFound and Unconfirmed do not create a model or mutate the queue", async () => {
  const f = await fixture(), original = await f.runtime.queue.gateway.read(f.scope), materialize = vi.spyOn(f.runtime.execution, "materializeCompaction");
  f.runtime.queue.failNextPrepare("beforeCommit"); await expect(f.runtime.queue.preview.prepare(f.request)).rejects.toThrow();
  expect((await f.runtime.queue.preview.prepareStatus(f.request)).status.type).toBe("notFound");
  f.runtime.queue.failNextPrepare("unavailable"); expect((await f.runtime.queue.preview.prepareStatus(f.request)).status.type).toBe("unconfirmed");
  expect(await f.runtime.queue.gateway.read(f.scope)).toEqual(original); expect(materialize).not.toHaveBeenCalled();
});
it("a concurrent Q edit rejects detached preparation before publishing a frozen record or proof", async () => {
  const f = await fixture(), actual = f.runtime.execution.materializeCompaction.bind(f.runtime.execution);
  let release!: () => void; const gate = new Promise<void>(resolve => {release = resolve;});
  vi.spyOn(f.runtime.execution, "materializeCompaction").mockImplementationOnce(async request => {const result = await actual(request); await gate; return result;});
  const work = f.runtime.queue.preview.prepare(f.request), result = work.catch(error => error);
  await vi.waitFor(() => expect(f.runtime.execution.materializeCompaction).toHaveBeenCalledOnce());
  const current = await f.runtime.queue.gateway.read(f.scope), latest = await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: current.revision,
    edit: {type: "add", inputId: crypto.randomUUID(), text: "tail"}}); release();
  expect(await result).toMatchObject({code: "storage", reason: {code: "DATA_SQL_DOMAIN"}});
  expect(await f.runtime.queue.gateway.read(f.scope)).toEqual(latest); expect((await f.runtime.queue.preview.prepareStatus(f.request)).status.type).toBe("notFound");
  await expect(f.runtime.gateways.submission.restoreWriting({conversationId: f.scope.conversationId, manuscriptId: f.scope.manuscriptId, requestId: f.request.submission.requestId})).rejects.toThrow();
});
it("uses native purpose facts rather than command-looking text", async () => {
  const f = await fixture("/compact");
  const ordinary = await f.runtime.queue.preview.prepare(f.request); expect(ordinary.preview.requestId).toBe(f.request.submission.requestId);
  await f.runtime.queue.preview.cancel({...f.scope, requestId: crypto.randomUUID(), inputId: f.inputId, previewRequestId: ordinary.preview.requestId, expectedRevision: ordinary.queue.revision});
  const q = await f.runtime.queue.gateway.read(f.scope), resumed = await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: q.revision, edit: {type: "resumeInput", inputId: f.inputId}});
  const submission = {...f.request.submission, requestId: crypto.randomUUID()}, compact = await f.runtime.compaction.gateway.prepareWritingQueued({inputId: f.inputId,
    expectedQueueRevision: resumed.revision, previousRequestId: null, submission, expectedRevision: 0});
  const wrongPurpose = {...f.request, expectedRevision: resumed.revision, submission};
  await expect(f.runtime.queue.preview.prepare(wrongPurpose)).rejects.toMatchObject({code: "storage"});
  await expect(f.runtime.queue.preview.prepareStatus(wrongPurpose)).rejects.toMatchObject({code: "storage"});
  expect(compact.preview.requestId).toBe(submission.requestId);
});
it("keeps an older preview with no typed prepare proof readonly and unconfirmed", async () => {
  const f = await fixture(); await f.runtime.queue.preview.prepare(f.request);
  const snapshot = f.runtime.queue.snapshot(); snapshot.preparations.clear(); f.runtime.queue.restore(snapshot);
  expect((await f.runtime.queue.preview.prepareStatus(f.request)).status).toMatchObject({type: "unconfirmed", detail: {safeCause: "legacy_prepare_without_intent_proof"}});
  await expect(f.runtime.queue.preview.prepare(f.request)).rejects.toMatchObject({code: "storage"});
  expect((await f.runtime.queue.preview.recover(f.scope))?.preview.requestId).toBe(f.request.submission.requestId);
});
it("invalid materialized data leaves neither a partial claim nor a typed preparation proof", async () => {
  const f = await fixture(), before = await f.runtime.queue.gateway.read(f.scope);
  vi.spyOn(f.runtime.execution, "materializeCompaction").mockResolvedValueOnce({preview: {} as never, commit: accept => accept()});
  await expect(f.runtime.queue.preview.prepare(f.request)).rejects.toThrow();
  expect(await f.runtime.queue.gateway.read(f.scope)).toEqual(before); expect((await f.runtime.queue.preview.prepareStatus(f.request)).status.type).toBe("notFound");
});
it("an older frozen record without a current pointer remains Legacy Unconfirmed and cannot gain a new proof", async () => {
  const f = await fixture(), prepared = await f.runtime.queue.preview.prepare(f.request);
  await f.runtime.queue.preview.cancel({...f.scope, requestId: crypto.randomUUID(), inputId: f.inputId, previewRequestId: prepared.preview.requestId,
    expectedRevision: prepared.queue.revision});
  const snapshot = f.runtime.queue.snapshot(); snapshot.preparations.clear(); f.runtime.queue.restore(snapshot);
  expect(await f.runtime.queue.preview.recover(f.scope)).toBeNull();
  const inspect = vi.spyOn(f.runtime.execution, "inspectFrozen"), record = {requestId: prepared.preview.requestId, ...f.scope};
  expect((await f.runtime.queue.preview.prepareStatus(f.request)).status).toMatchObject({type: "unconfirmed", detail: {safeCause: "legacy_prepare_without_intent_proof"}});
  expect(inspect).toHaveBeenCalledWith(record); await expect(f.runtime.queue.preview.prepare(f.request)).rejects.toMatchObject({code: "storage"});
  expect(f.runtime.execution.inspectFrozen(record)).toEqual(prepared.preview); expect(f.runtime.queue.snapshot().preparations.size).toBe(0);
  expect(() => f.runtime.execution.inspectFrozen({...record, manuscriptId: crypto.randomUUID()})).toThrow();
});
it.each([false, true])("fresh queued admission resumes only the current global pause and exact replay preserves a later Pause (paused=%s)", async paused => {
  const f = await fixture(), prepared = await f.runtime.queue.preview.prepare(f.request), tailId = crypto.randomUUID();
  const appended = await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: prepared.queue.revision,
    edit: {type: "add", inputId: tailId, text: "individually held tail"}});
  await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: appended.revision, edit: {type: "pauseInput", inputId: tailId}});
  if (paused) f.runtime.queue.restore(f.runtime.queue.snapshot());
  const current = (await f.runtime.queue.preview.recover(f.scope))!;
  await f.runtime.gateways.workspace.prepare({scope: {...f.scope, requestId: prepared.preview.requestId, contextSnapshotId: prepared.preview.contextSnapshotId}, buffers: []});
  const start = {inputId: f.inputId, expectedRevision: current.queue.revision, start: {...f.scope, requestId: prepared.preview.requestId,
    contextSnapshotId: prepared.preview.contextSnapshotId, model: prepared.preview.input.model, mode: "deepSeekNonThinking" as const}};
  const execute = vi.spyOn(f.runtime.execution.submission, "startWriting"), admitted = await f.runtime.queue.preview.start(start);
  const after = await f.runtime.queue.gateway.read(f.scope);
  expect(after).toMatchObject({revision: current.queue.revision + (paused ? 2 : 1), paused: false, preparingInputId: null});
  expect(after.inputs).toEqual(current.queue.inputs.filter(input => input.inputId !== f.inputId)); expect(after.inputs[0]?.paused).toBe(true);
  const laterPause = await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: after.revision, edit: {type: "pause"}});
  expect(await f.runtime.queue.preview.start(start)).toEqual({...admitted, alreadyStarted: true}); expect(await f.runtime.queue.gateway.read(f.scope)).toEqual(laterPause);
  await expect(f.runtime.queue.preview.start({...start, expectedRevision: laterPause.revision})).rejects.toMatchObject({code: "conflict"});
  expect(execute).toHaveBeenCalledOnce(); expect(await f.runtime.queue.gateway.read(f.scope)).toEqual(laterPause);
});
