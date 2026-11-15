import { afterEach, expect, it, vi } from "vitest";
import { WritingDiffDemoStorage } from "./WritingDiffDemoStorage";
import { WritingDiffDemoAgentStorage } from "./WritingDiffDemoAgentStorage";
import { WritingDiffDemoSubmission } from "./WritingDiffDemoSubmission";
import { WritingDiffDemoQueue } from "./WritingDiffDemoQueue";
import { createWritingDiffDemoRuntime } from "./writingDiffDemoRuntime";
import { initialChanges } from "../../../core/features/agent/WritingDiffFixtures";

const manuscriptId = "00000000-0000-4000-8000-000000000003";
const conversationId = "00000000-0000-4000-8000-000000000001";
const live: { execution: WritingDiffDemoSubmission; agent: WritingDiffDemoAgentStorage }[] = [];
afterEach(() => { for (const item of live.splice(0)) { item.execution.dispose(); item.agent.history.dispose(); } vi.useRealTimers(); });
async function fixture() {
  const storage = new WritingDiffDemoStorage(manuscriptId, initialChanges);
  const agent = new WritingDiffDemoAgentStorage(storage, { admission: { scope: { manuscriptId, conversationId,
    requestId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID() }, turnId: crypto.randomUUID() }, changes: initialChanges });
  await agent.initialized;
  const execution = new WritingDiffDemoSubmission(storage, agent, manuscriptId); execution.delayMs = 100_000;
  live.push({ execution, agent });
  const files = await storage.files.listFiles(manuscriptId);
  const buffers = await Promise.all(files.map(async file => ({ path: file.relativePath, expectedDiskSha256: file.sha256,
    sessionId: crypto.randomUUID(), revision: crypto.randomUUID(), text: (await storage.files.readFile(manuscriptId, file.relativePath)).content })));
  const current = buffers.find(file => file.path === "main.tex")!;
  const submission = { conversationId, requestId: crypto.randomUUID(), text: "Use latest source", context: { manuscriptId,
    files: [{ relativePath: current.path, expectedDiskSha256: current.expectedDiskSha256, buffer: { revision: current.revision, text: current.text } }] } };
  return { storage, agent, execution, buffers, submission };
}
async function startMultipleResponses() {
  const f = await fixture();
  f.execution.multipleResponses = true; f.execution.delayMs = 3000;
  const preview = await f.execution.submission.prepareWriting(f.submission);
  const scope = { manuscriptId, conversationId, requestId: preview.requestId, contextSnapshotId: preview.contextSnapshotId };
  await f.execution.workspace.prepare({ scope, buffers: f.buffers });
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  await f.execution.submission.startWriting({ ...scope, model: preview.input.model, mode: "deepSeekNonThinking" });
  const read = async () => (await f.agent.history.gateway.readHistory({ conversationId, afterSeq: 0, limit: 500 }))
    .events.filter(event => event.payload.turnId === preview.turnId);
  return { ...f, scope, preview, read };
}
it.each([false,true])("mock stop pauses the queue before finishing and a stale stop cannot pause a newer turn (history only: %s)", async historyOnly => {
  const f = await fixture();
  const runtime = createWritingDiffDemoRuntime(f.storage, f.agent, manuscriptId, conversationId);
  runtime.execution.historyOnly = historyOnly;
  runtime.execution.delayMs = 100_000;
  try {
    const start = async () => {
      const preview = await runtime.gateways.submission.prepareWriting({ ...f.submission, requestId: crypto.randomUUID() });
      const scope = { manuscriptId, conversationId, requestId: preview.requestId, contextSnapshotId: preview.contextSnapshotId };
      await runtime.gateways.workspace.prepare({ scope, buffers: f.buffers });
      await runtime.gateways.submission.startWriting({ ...scope, model: preview.input.model, mode: "deepSeekNonThinking" });
      return preview.turnId;
    };
    const oldTurn = await start();
    const scope = { manuscriptId, conversationId };
    await runtime.queue.gateway.edit({ ...scope, requestId: crypto.randomUUID(), expectedRevision: 0,
      edit: { type: "add", inputId: crypto.randomUUID(), text: "Retained after stop" } });
    await runtime.gateways.turns.interrupt({ conversationId, turnId: oldTurn });
    const stopped = await runtime.queue.gateway.read(scope);
    expect(stopped).toMatchObject({ paused: true, inputs: [{ text: "Retained after stop" }] });
    await runtime.gateways.turns.interrupt({ conversationId, turnId: oldTurn });
    expect(await runtime.queue.gateway.read(scope)).toEqual(stopped);
    await runtime.queue.gateway.edit({ ...scope, requestId: crypto.randomUUID(), expectedRevision: stopped.revision, edit: { type: "resume" } });
    const newTurn = await start();
    expect(newTurn).not.toBe(oldTurn);
    const resumed = await runtime.queue.gateway.read(scope);
    await runtime.gateways.turns.interrupt({ conversationId, turnId: oldTurn });
    expect(await runtime.queue.gateway.read(scope)).toEqual(resumed);
  } finally { runtime.dispose(); }
});
it("publishes multiple tool/text responses in one turn and retains both edits in its frozen result", async () => {
  const f = await startMultipleResponses();
  const original = await f.storage.files.readFile(manuscriptId, "main.tex");
  await vi.advanceTimersByTimeAsync(1200);
  const first = await f.read();
  expect(first.some(event => event.payload.type === "textDelta" && event.payload.text.includes("第一次修改"))).toBe(true);
  expect(first.some(event => event.payload.type === "turnFinished")).toBe(false);
  await vi.advanceTimersByTimeAsync(600);
  const second = await f.read();
  expect(second.slice(0, first.length)).toEqual(first);
  expect(second.some(event => event.payload.type === "textDelta" && event.payload.text.includes("第二次修改"))).toBe(true);
  expect(await f.storage.files.readFile(manuscriptId, "main.tex")).toEqual(original);
  await vi.advanceTimersByTimeAsync(1200);
  const review = await f.agent.reviews.review(f.scope);
  const completed = await f.read();
  expect(completed.filter(event => event.payload.type === "turnFinished")).toHaveLength(1);
  expect(completed.filter(event => event.payload.type === "itemStarted" && event.payload.kind.type === "assistantMessage")).toHaveLength(4);
  expect(completed.filter(event => event.payload.type === "itemStarted" && event.payload.kind.type === "toolCall"
    && event.payload.kind.name === "research_apply_patch")).toHaveLength(2);
  const change = review.changes.find(item => item.path === "main.tex")!;
  const file = await f.agent.reviews.readFile({ binding: review.binding, path: "main.tex", before: change.before, after: change.after });
  expect(file.before).toEqual({ kind: "text", text: original.content });
  expect(file.after).toMatchObject({ kind: "text", text: expect.stringContaining("keywords updated") });
  expect(file.after).toMatchObject({ kind: "text", text: expect.stringContaining("\\keywords{research, agent, reproducibility}") });
  expect(file.after).toMatchObject({ kind: "text", text: expect.stringContaining("\\begin{abstract}") });
  expect(await f.storage.files.readFile(manuscriptId, "main.tex")).toEqual(original);
});
it.each(["interrupt", "dispose"] as const)("stops remaining response steps after %s", async action => {
  const f = await startMultipleResponses();
  await vi.advanceTimersByTimeAsync(1200);
  if (action === "interrupt") await f.execution.turns.interrupt({ conversationId, turnId: f.preview.turnId });
  else f.execution.dispose();
  const stopped = await f.read();
  await vi.advanceTimersByTimeAsync(10_000);
  expect(await f.read()).toEqual(stopped);
  expect(stopped.some(event => event.payload.type === "textDelta" && event.payload.text.includes("第二次修改"))).toBe(false);
});
it("reschedules only remaining steps without replaying already returned responses", async () => {
  const f = await startMultipleResponses();
  await vi.advanceTimersByTimeAsync(600);
  const first = await f.read();
  f.execution.reschedule(500);
  await vi.advanceTimersByTimeAsync(500);
  await f.agent.reviews.review(f.scope);
  const completed = await f.read();
  expect(completed.slice(0, first.length)).toEqual(first);
  expect(completed.filter(event => event.payload.type === "turnFinished")).toHaveLength(1);
  expect(completed.filter(event => event.payload.type === "itemStarted" && event.payload.kind.type === "assistantMessage")).toHaveLength(4);
});
it("freezes selected model context and all execution buffers, then recovers a canceled start receipt", async () => {
  const f = await fixture();
  const [preview, repeatedPreview] = await Promise.all([f.execution.submission.prepareWriting(f.submission), f.execution.submission.prepareWriting(f.submission)]);
  expect(repeatedPreview).toEqual(preview);
  const scope = { conversationId, manuscriptId, requestId: preview.requestId, contextSnapshotId: preview.contextSnapshotId };
  const buffers = f.buffers.map(file => file.path === "sections/introduction.tex" ? { ...file, text: `${file.text}\n% unsaved input` } : file);
  const workspace = await f.execution.workspace.prepare({ scope, buffers });
  expect(workspace.files).toHaveLength(3);
  expect(preview.input.context).toHaveLength(1);
  expect(await f.execution.workspace.prepare({ scope, buffers })).toEqual(workspace);
  buffers[0]!.text = "caller changed after capture";
  expect(await f.execution.workspace.restore(scope)).toEqual(workspace);
  const start = { ...scope, model: preview.input.model, mode: "deepSeekNonThinking" as const };
  const admitted = await f.execution.submission.startWriting(start);
  expect(admitted.alreadyStarted).toBe(false);
  await expect(f.execution.turns.interrupt({ conversationId: crypto.randomUUID(), turnId: preview.turnId })).rejects.toThrow();
  expect((await f.execution.turns.interrupt({ conversationId, turnId: preview.turnId })).cancellationRequested).toBe(true);
  const restored = await f.execution.submission.startWriting(start);
  expect(restored).toMatchObject({ alreadyStarted: true, outcome: "canceled" });
  const terminal = await f.agent.gateways.delivery.sessions.readHistory({ conversationId, afterSeq: 0, limit: 100 });
  expect(terminal.events.at(-1)?.payload).toMatchObject({ type: "turnFinished", turnId: preview.turnId, outcome: "canceled" });
  // Native proof of an empty canceled execution settles production delivery.
  await expect(f.agent.gateways.delivery.deliveries.acknowledge({ scope, turnId: preview.turnId })).resolves.toBeUndefined();
});

it("retries queued preparation, start and cancellation by exact request without consuming another input", async () => {
  const f = await fixture(), queue = new WritingDiffDemoQueue(f.execution.submission, undefined, request => f.execution.materializeCompaction(request), scope => f.execution.inspectFrozen(scope),
    {validate: request => f.execution.validateQueuedStart(request), inspect: (request, preview) => f.execution.inspectStarted(request, preview)}), scope = { manuscriptId, conversationId };
  const inputId = crypto.randomUUID();
  const added = await queue.gateway.edit({ ...scope, requestId: crypto.randomUUID(), expectedRevision: 0,
    edit: { type: "add", inputId, text: f.submission.text } });
  const request = { inputId, expectedRevision: added.revision, submission: f.submission };
  const [first, duplicate] = await Promise.all([queue.preview.prepare(request), queue.preview.prepare(request)]);
  expect(duplicate).toEqual(first);
  await expect(queue.preview.prepare({ ...request, submission: { ...f.submission, text: "conflicting retry" } })).rejects.toMatchObject({code: "storage", reason: {code: "DATA_SQL_DOMAIN", conversationId, manuscriptId, inputId, previewRequestId: f.submission.requestId, operationId: f.submission.requestId}});
  const preparedScope = { ...scope, requestId: first.preview.requestId, contextSnapshotId: first.preview.contextSnapshotId };
  await f.execution.workspace.prepare({ scope: preparedScope, buffers: f.buffers });
  const start = { inputId, expectedRevision: first.queue.revision,
    start: { ...preparedScope, model: first.preview.input.model, mode: "deepSeekNonThinking" as const } };
  const [started, repeated] = await Promise.all([queue.preview.start(start), queue.preview.start(start)]);
  expect(repeated).toEqual(started);
  expect((await queue.gateway.read(scope)).inputs).toHaveLength(0);
  const nextInput = crypto.randomUUID();
  const next = await queue.gateway.edit({ ...scope, requestId: crypto.randomUUID(), expectedRevision: first.queue.revision + 1,
    edit: { type: "add", inputId: nextInput, text: "Next queued source" } });
  const pending = await queue.preview.prepare({ inputId: nextInput, expectedRevision: next.revision,
    submission: { ...f.submission, requestId: crypto.randomUUID(), text: "Next queued source" } });
  const cancel = { ...scope, requestId: crypto.randomUUID(), expectedRevision: pending.queue.revision, inputId: nextInput, previewRequestId: pending.preview.requestId };
  const canceled = await queue.preview.cancel(cancel);
  expect(await queue.preview.cancel(cancel)).toEqual(canceled);
  expect(canceled).toMatchObject({ paused: true, preparingInputId: null, inputs: [{ inputId: nextInput, text: "Next queued source" }] });
});

it("retains submission-time disk sources when an external actor edits during model execution", async () => {
  const f = await fixture();
  const preview = await f.execution.submission.prepareWriting(f.submission);
  const scope = { manuscriptId, conversationId, requestId: preview.requestId, contextSnapshotId: preview.contextSnapshotId };
  await f.execution.workspace.prepare({ scope, buffers: f.buffers });
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  await f.execution.submission.startWriting({ ...scope, model: preview.input.model, mode: "deepSeekNonThinking" });
  const changed = f.buffers[1]!;
  await f.storage.externalEdit(changed.path, "external actor changed the formal file");
  await vi.advanceTimersByTimeAsync(f.execution.delayMs);
  const review = await f.agent.reviews.review(scope);
  const proposal = await f.agent.gateways.delivery.proposals.publish(review);
  const disk = await f.storage.files.readFile(manuscriptId, changed.path);
  // Even the latest disk hash cannot authorize output produced from an older copy.
  const buffers = f.buffers.map(buffer => buffer.path === changed.path ? { ...buffer, expectedDiskSha256: disk.sha256 } : buffer);
  await expect(f.agent.gateways.application.applications.execute({ action: "begin",
    scope: { manuscriptId, conversationId, proposalId: proposal.proposalId, operationId: crypto.randomUUID() }, buffers })).rejects.toThrow("版本已过期");
  expect((await f.storage.files.readFile(manuscriptId, changed.path)).content).toBe(disk.content);
  expect(await f.agent.gateways.delivery.proposals.latest(scope)).toMatchObject({ status: "pending" });
});

it("accepts a result after the exact frozen working buffer has been formally saved", async () => {
  const f = await fixture(), selected = f.buffers.find(buffer => buffer.path === "main.tex")!;
  const text = `${selected.text}\n% unsaved input at submission`;
  const input = { ...f.submission, context: { ...f.submission.context, files: [{ relativePath: selected.path,
    expectedDiskSha256: selected.expectedDiskSha256, buffer: { revision: selected.revision, text } }] } };
  const preview = await f.execution.submission.prepareWriting(input);
  const scope = { manuscriptId, conversationId, requestId: preview.requestId, contextSnapshotId: preview.contextSnapshotId };
  const buffers = f.buffers.map(buffer => buffer.path === selected.path ? { ...buffer, text } : buffer);
  await f.execution.workspace.prepare({ scope, buffers });
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  await f.execution.submission.startWriting({ ...scope, model: preview.input.model, mode: "deepSeekNonThinking" });
  const saved = await f.storage.files.saveFile({ manuscriptId, relativePath: selected.path, content: text, expectedSha256: selected.expectedDiskSha256 });
  await vi.advanceTimersByTimeAsync(f.execution.delayMs);
  const review = await f.agent.reviews.review(scope), proposal = await f.agent.gateways.delivery.proposals.publish(review);
  const current = buffers.map(buffer => buffer.path === selected.path ? { ...buffer, expectedDiskSha256: saved.sha256 } : buffer);
  await expect(f.agent.gateways.application.applications.execute({ action: "begin",
    scope: { manuscriptId, conversationId, proposalId: proposal.proposalId, operationId: crypto.randomUUID() }, buffers: current })).resolves.toMatchObject({ status: "applying" });
  expect((await f.agent.reviews.review(scope))).toEqual(review);
});
it("inspects original per-item events and frozen source without rebuilding from current input", async () => {
  const f = await startMultipleResponses();
  const events = await f.read();
  const first = events.find(event => event.payload.type === "itemStarted" && event.payload.kind.type === "userMessage")!;
  if (!("itemId" in first.payload)) throw new Error("missing fixture item");
  const request = { conversationId, turnId: f.preview.turnId, itemId: first.payload.itemId };
  const original = f.agent.history.inspectMessage(request);
  expect(original.every(event => event.payload.turnId === request.turnId && "itemId" in event.payload && event.payload.itemId === request.itemId)).toBe(true);
  expect(original.some(event => event.payload.type === "textDelta" && event.payload.text === f.submission.text)).toBe(true);
  const frozen = f.execution.inspectSubmission(conversationId, f.preview.turnId);
  f.submission.text = "edited after sending";
  f.submission.context.files[0]!.buffer.text = "new source";
  expect(f.execution.inspectSubmission(conversationId, f.preview.turnId)).toEqual(frozen);
  expect(frozen).toMatchObject({ requestId: f.preview.requestId, input: { text: "Use latest source" } });
  expect(() => f.agent.history.inspectMessage({ ...request, conversationId: crypto.randomUUID() })).toThrow();
  expect(() => f.agent.history.inspectMessage({ ...request, itemId: crypto.randomUUID() })).toThrow();
});

it("skips an individually paused entry and admits only the eligible entry", async () => {
  const f = await fixture(), queue = new WritingDiffDemoQueue(f.execution.submission, undefined, request => f.execution.materializeCompaction(request), scope => f.execution.inspectFrozen(scope),
    {validate: request => f.execution.validateQueuedStart(request), inspect: (request, preview) => f.execution.inspectStarted(request, preview)}), scope = {manuscriptId, conversationId};
  const firstId = crypto.randomUUID(), secondId = crypto.randomUUID();
  await queue.gateway.edit({...scope, requestId: crypto.randomUUID(), expectedRevision: 0,
    edit: {type: "add", inputId: firstId, text: "Held message"}});
  await queue.gateway.edit({...scope, requestId: crypto.randomUUID(), expectedRevision: 1,
    edit: {type: "add", inputId: secondId, text: f.submission.text}});
  const pause = {...scope, requestId: crypto.randomUUID(), expectedRevision: 2, edit: {type: "pauseInput" as const, inputId: firstId}};
  const held = await queue.gateway.edit(pause);
  expect(await queue.gateway.edit(pause)).toEqual(held);
  expect(held.paused).toBe(false);
  await expect(queue.preview.prepare({inputId: firstId, expectedRevision: held.revision,
    submission: {...f.submission, text: "Held message"}})).rejects.toMatchObject({code: "storage", reason: {code: "DATA_SQL_DOMAIN", conversationId, manuscriptId, inputId: firstId, previewRequestId: f.submission.requestId, operationId: f.submission.requestId}});
  const prepared = await queue.preview.prepare({inputId: secondId, expectedRevision: held.revision, submission: f.submission});
  const preparedScope = {...scope, requestId: prepared.preview.requestId, contextSnapshotId: prepared.preview.contextSnapshotId};
  await f.execution.workspace.prepare({scope: preparedScope, buffers: f.buffers});
  await queue.preview.start({inputId: secondId, expectedRevision: prepared.queue.revision,
    start: {...preparedScope, model: prepared.preview.input.model, mode: "deepSeekNonThinking"}});
  const remaining = await queue.gateway.read(scope);
  expect(remaining.inputs).toEqual([{inputId: firstId, text: "Held message", userInput: [], paused: true}]);
  const resumed = await queue.gateway.edit({...scope, requestId: crypto.randomUUID(), expectedRevision: remaining.revision,
    edit: {type: "resumeInput", inputId: firstId}});
  expect(resumed.inputs[0]?.paused).toBe(false);
});

it("resumes only the chosen message after a queue-wide stop", async () => {
  const f = await fixture(), queue = new WritingDiffDemoQueue(f.execution.submission, undefined, request => f.execution.materializeCompaction(request), scope => f.execution.inspectFrozen(scope),
    {validate: request => f.execution.validateQueuedStart(request), inspect: (request, preview) => f.execution.inspectStarted(request, preview)}), scope = {manuscriptId, conversationId};
  const firstId = crypto.randomUUID(), secondId = crypto.randomUUID();
  for (const [revision, inputId] of [firstId, secondId].entries()) {
    await queue.gateway.edit({...scope, requestId: crypto.randomUUID(), expectedRevision: revision,
      edit: {type: "add", inputId, text: "message"}});
  }
  queue.pauseForStop(scope);
  const resumed = await queue.gateway.edit({...scope, requestId: crypto.randomUUID(), expectedRevision: 3,
    edit: {type: "resumeInput", inputId: secondId}});
  expect(resumed.paused).toBe(false);
  expect(resumed.inputs.map(input => input.paused)).toEqual([true, false]);
});
