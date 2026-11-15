import {afterEach, expect, it, vi} from "vitest";
import {agentWritingCompactionStartRequest, decodeAgentWritingPrepareRequest,
  type AgentWritingPrepareRequest, type AgentWritingPrepareStatusResponse, type AgentWritingCompactionPrepareStatusResponse,
  type AgentContextText} from "@research-assistant/core/agent";
import {WritingDiffDemoStorage} from "./WritingDiffDemoStorage";
import {WritingDiffDemoAgentStorage} from "./WritingDiffDemoAgentStorage";
import {createWritingDiffDemoRuntime} from "./writingDiffDemoRuntime";
type Side = "ordinary" | "manual";
const cleanups: (() => void)[] = [];
afterEach(() => {cleanups.splice(0).forEach(cleanup => cleanup()); vi.restoreAllMocks();});
async function fixture(side: Side) {
  const scope = {manuscriptId: crypto.randomUUID(), conversationId: crypto.randomUUID()};
  const storage = new WritingDiffDemoStorage(scope.manuscriptId, [{path: "main.tex", before: "original file", after: "original file", kind: "modified", additions: 0, deletions: 0}]);
  const agent = new WritingDiffDemoAgentStorage(storage, undefined), runtime = createWritingDiffDemoRuntime(storage, agent, scope.manuscriptId, scope.conversationId);
  runtime.execution.historyOnly = true; cleanups.push(() => runtime.dispose());
  const disk = await storage.files.readFile(scope.manuscriptId, "main.tex");
  const request: AgentWritingPrepareRequest = {conversationId: scope.conversationId, requestId: crypto.randomUUID(),
    text: side === "ordinary" ? "original request" : "/compact", context: {manuscriptId: scope.manuscriptId,
      files: [{relativePath: "main.tex", expectedDiskSha256: disk.sha256}]}, sessionConfig: {maxBytes: 1024}, turnConfig: {fallbackFilenames: ["AGENTS.md"]}};
  const methods = (value: typeof runtime) => ({
    prepare: (input = request) => side === "ordinary" ? value.gateways.submission.prepareWriting(input)
      : value.compaction.gateway.prepareWriting({request: input, expectedRevision: 0}).then(prepared => prepared.preview),
    status: (input = request): Promise<AgentWritingPrepareStatusResponse | AgentWritingCompactionPrepareStatusResponse> => side === "ordinary"
      ? value.gateways.submission.prepareWritingStatus(input) : value.compaction.gateway.prepareWritingStatus({request: input, expectedRevision: 0}),
  });
  const fail = (failure: "beforeCommit" | "afterCommit" | "unavailable" | null) => side === "ordinary"
    ? runtime.execution.failNextPrepare(failure) : runtime.compaction.failNext("prepare", failure);
  const reopen = () => {
    const records = runtime.execution.snapshot(), compaction = runtime.compaction.snapshot(), queue = runtime.queue.snapshot(), events = agent.history.snapshot();
    const nextAgent = new WritingDiffDemoAgentStorage(storage, undefined), next = createWritingDiffDemoRuntime(storage, nextAgent, scope.manuscriptId, scope.conversationId);
    nextAgent.history.restore(events); next.execution.restore(records); next.queue.restore(queue); next.compaction.restore(compaction);
    cleanups.push(() => next.dispose()); return {runtime: next, agent: nextAgent, ...methods(next)};
  };
  return {side, scope, storage, agent, runtime, request, fail, reopen, ...methods(runtime)};
}
function committed(response: AgentWritingPrepareStatusResponse | AgentWritingCompactionPrepareStatusResponse) {
  expect(response.status.type).toBe("committed"); if (response.status.type !== "committed") throw new Error("missing original proof");
  return response.status;
}
const sides = ["ordinary", "manual"] as const;
it.each(sides)("%s before-commit failure leaves no record and status does not consume the fault or read sources", async side => {
  const f = await fixture(side), read = vi.spyOn(f.storage.files, "readFile"); f.fail("beforeCommit");
  expect((await f.status()).status).toEqual({type: "notFound"}); expect(read).not.toHaveBeenCalled();
  await expect(f.prepare()).rejects.toBeDefined(); expect((await f.status()).status.type).toBe("notFound");
  expect(f.runtime.execution.snapshot()).toHaveLength(0); expect(f.runtime.compaction.snapshot().records.size).toBe(0);
  expect(read).not.toHaveBeenCalled(); expect((await f.agent.history.gateway.readHistory({conversationId: f.scope.conversationId, afterSeq: 0, limit: 100})).events).toHaveLength(0);
});
it.each(sides)("%s lost preparation ACK retains full request/proof through reopen and never reads current dependencies again", async side => {
  const f = await fixture(side); f.fail("afterCommit"); await expect(f.prepare()).rejects.toBeDefined();
  const original = committed(await f.status()), reopened = f.reopen(), read = vi.spyOn(f.storage.files, "readFile").mockRejectedValue(new Error("current source removed"));
  const freeze = vi.spyOn(reopened.runtime.execution.resources, "freeze").mockRejectedValue(new Error("current model resources unavailable"));
  expect(committed(await reopened.status())).toEqual(original); const prepared = await reopened.prepare();
  expect(prepared.requestId).toBe(f.request.requestId); expect(committed(await reopened.status())).toEqual(original);
  expect(read).not.toHaveBeenCalled(); expect(freeze).not.toHaveBeenCalled();
  expect((await reopened.agent.history.gateway.readHistory({conversationId: f.scope.conversationId, afterSeq: 0, limit: 100})).events).toHaveLength(0);
});
it.each(sides)("%s compares every frozen request field before mutable source work", async side => {
  const f = await fixture(side); await f.prepare(); const read = vi.spyOn(f.storage.files, "readFile").mockRejectedValue(new Error("must not rematerialize"));
  const changes: AgentWritingPrepareRequest[] = [{...f.request, text: `${f.request.text} `}, {...f.request, model: {providerId: "openai", modelId: "gpt-5.6-terra", serviceTier: "fast"}},
    {...f.request, retrieval: {traceId: "different-trace", query: "different query"}}, {...f.request, sessionConfig: {maxBytes: 2048}},
    {...f.request, turnConfig: {fallbackFilenames: ["OTHER.md"]}}, {...f.request, context: {...f.request.context, files: [{...f.request.context.files[0]!, buffer: {revision: crypto.randomUUID(), text: "later unsaved input"}}]}}];
  for (const changed of changes) {
    await expect(f.prepare(changed)).rejects.toMatchObject({code: "confirmationMismatch"});
    expect((await f.status(changed)).status.type).toBe("unconfirmed");
  }
  expect(read).not.toHaveBeenCalled(); expect((await f.status()).intent).toEqual(side === "ordinary" ? decodeAgentWritingPrepareRequest(f.request) : {request: decodeAgentWritingPrepareRequest(f.request), expectedRevision: 0});
});
it.each(sides)("%s legacy records and corrupt proofs remain Unconfirmed without being upgraded by retry", async side => {
  const f = await fixture(side); await f.prepare(); const original = committed(await f.status());
  if (side === "ordinary") {
    f.runtime.execution.restore(f.runtime.execution.snapshot().map(([key, value]) => {const {preparation: _proof, purpose: _purpose, ...legacy} = value; return [key, {...legacy, started: true, outcome: "completed" as const}] as const;}));
  } else {
    const snapshot = f.runtime.compaction.snapshot(); for (const [key, value] of snapshot.records) {const {preparation: _proof, ...legacy} = value; snapshot.records.set(key, {...legacy, started: true, outcome: "completed"});} f.runtime.compaction.restore(snapshot);
  }
  const read = vi.spyOn(f.storage.files, "readFile").mockRejectedValue(new Error("current source unavailable"));
  expect((await f.status()).status.type).toBe("unconfirmed"); await expect(f.prepare()).rejects.toMatchObject({code: "recoveryRequired"}); expect(read).not.toHaveBeenCalled();
  expect(original.proof.family).toBe("agent.writing.submission.prepare"); expect(original.proof.sqlFence).toBe("0");
});
it.each(sides)("%s refuses empty fresh input and preserves supplemental-only native grant scope", async side => {
  const f = await fixture(side), empty = {...f.request, context: {...f.request.context, files: []}};
  await expect(f.prepare(empty)).rejects.toMatchObject({code: "invalidInput"}); expect((await f.status(empty)).status.type).toBe("notFound");
  const noteId = crypto.randomUUID(), resource = {kind: "note" as const, resourceId: noteId}, hash = "a".repeat(64);
  f.runtime.execution.contextSources = async () => [{sourceId: `resource:${hash}`, revision: `grant:${hash}`, kind: "note", resourceId: noteId,
    anchor: JSON.stringify({supplementalContext: true, content: "resourceGrant", resource, contextAccess: "readOnly"}), text: "authorized supplemental note"}];
  const request = {...empty, resources: [resource]}; expect((await f.prepare(request)).input.context).toHaveLength(1); expect((await f.status(request)).status.type).toBe("committed");
});
it.each(sides)("%s does not accept foreign primary anchors or stale first-capture file hashes", async side => {
  const f = await fixture(side), stale = {...f.request, context: {...f.request.context, files: [{...f.request.context.files[0]!, expectedDiskSha256: "a".repeat(64)}]}};
  await expect(f.prepare(stale)).rejects.toMatchObject({code: "conflict"}); expect((await f.status(stale)).status.type).toBe("notFound");
  f.runtime.execution.contextSources = async () => [{sourceId: "foreign", revision: "foreign", kind: "manuscript", resourceId: "foreign.tex", anchor: JSON.stringify({manuscriptId: crypto.randomUUID()}), text: "foreign file"}];
  const foreign = {...f.request, context: {...f.request.context, files: []}, resources: [{kind: "note" as const, resourceId: crypto.randomUUID()}]};
  await expect(f.prepare(foreign)).rejects.toMatchObject({code: "invalidInput"}); expect((await f.status(foreign)).status.type).toBe("notFound");
});
it.each(sides)("%s accepts Native's case-insensitive disk hash while retaining the original intent bytes", async side => {
  const f = await fixture(side), request = {...f.request, context: {...f.request.context, files: [{...f.request.context.files[0]!, expectedDiskSha256: f.request.context.files[0]!.expectedDiskSha256.toUpperCase()}]}};
  await f.prepare(request); const response = await f.status(request);
  expect(response.intent).toEqual(side === "ordinary" ? decodeAgentWritingPrepareRequest(request) : {request: decodeAgentWritingPrepareRequest(request), expectedRevision: 0});
  expect(response.status.type).toBe("committed");
});
it("manual history W and purpose are frozen facts, not the later queue revision", async () => {
  const f = await fixture("manual"), prepared = await f.runtime.compaction.gateway.prepareWriting({request: f.request, expectedRevision: 0});
  const original = committed(await f.status()); await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: 0, edit: {type: "add", inputId: crypto.randomUUID(), text: "later queued input"}});
  expect(committed(await f.status())).toEqual(original); expect((await f.runtime.compaction.gateway.prepareWritingStatus({request: f.request, expectedRevision: 1})).status.type).toBe("unconfirmed");
  await expect(f.runtime.gateways.submission.prepareWriting(f.request)).rejects.toMatchObject({code: "confirmationMismatch"});
  expect((await f.runtime.gateways.submission.prepareWritingStatus(f.request)).status.type).toBe("unconfirmed");
  const execute = vi.spyOn(f.runtime.compaction["gateway"], "start"), request = agentWritingCompactionStartRequest(prepared); await f.runtime.compaction.gateway.start(request);
  expect(await f.runtime.compaction.gateway.prepareWriting({request: f.request, expectedRevision: 0})).toEqual(prepared);
  expect(committed(await f.status())).toEqual(original); expect(execute).toHaveBeenCalledOnce();
});
it("ordinary records cannot be adopted by manual or queued purpose and queued receipts never become ordinary proofs", async () => {
  const f = await fixture("ordinary"); await f.prepare();
  await expect(f.runtime.compaction.gateway.prepareWriting({request: {...f.request, text: "/compact"}, expectedRevision: 0})).rejects.toMatchObject({code: "confirmationMismatch"});
  expect((await f.runtime.compaction.gateway.prepareWritingStatus({request: f.request, expectedRevision: 0})).status.type).toBe("unconfirmed");
  const queued = {...f.request, requestId: crypto.randomUUID()}, inputId = crypto.randomUUID();
  await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: 0, edit: {type: "add", inputId, text: queued.text}});
  await f.runtime.queue.preview.prepare({inputId, expectedRevision: 1, submission: queued});
  expect((await f.runtime.gateways.submission.prepareWritingStatus(queued)).status.type).toBe("unconfirmed");
  await expect(f.runtime.gateways.submission.prepareWriting(queued)).rejects.toMatchObject({code: "confirmationMismatch"});
});
it.each(sides)("%s corrupt-but-well-typed proof metadata cannot be read or replayed as success", async side => {
  const f = await fixture(side); await f.prepare();
  if (side === "ordinary") f.runtime.execution.restore(f.runtime.execution.snapshot().map(([key, value]) => [key, {...value, preparation: {...value.preparation!, proof: {...value.preparation!.proof, sqlFence: "1"}}}] as const));
  else {const snapshot = f.runtime.compaction.snapshot(); for (const [key, value] of snapshot.records) snapshot.records.set(key, {...value, preparation: {...value.preparation!, proof: {...value.preparation!.proof, sqlFence: "1"}}}); f.runtime.compaction.restore(snapshot);}
  expect((await f.status()).status.type).toBe("unconfirmed"); await expect(f.prepare()).rejects.toMatchObject({code: "recoveryRequired"});
});
it.each(sides)("%s consumes a retrieval trace only with the original record, never again after ACK loss or reopening", async side => {
  const f = await fixture(side), retrieval = {traceId: crypto.randomUUID(), query: "original query"}, request = {...f.request, retrieval};
  let consumed = false;
  const context: AgentContextText[] = [{sourceId: `retrieval:${retrieval.traceId}`, revision: `trace:${retrieval.traceId}`, kind: "retrieval", resourceId: retrieval.traceId,
    anchor: JSON.stringify({manuscriptId: f.scope.manuscriptId, traceId: retrieval.traceId}), text: "original traced context"}];
  const consume = vi.fn((accept: () => void) => {if (consumed) throw {code: "conflict"}; accept(); consumed = true;});
  const readTrace = vi.fn(async (input: AgentWritingPrepareRequest) => {expect(input.retrieval).toEqual(retrieval); if (consumed) throw {code: "conflict"}; return {context, commit: consume};});
  f.runtime.execution.retrievalPreparation = readTrace; f.fail("afterCommit"); await expect(f.prepare(request)).rejects.toBeDefined();
  expect(consumed).toBe(true); const original = committed(await f.status(request)), reopened = f.reopen(); reopened.runtime.execution.retrievalPreparation = readTrace;
  expect(committed(await reopened.status(request))).toEqual(original); expect((await reopened.prepare(request)).input.context).toContainEqual(context[0]);
  expect(readTrace).toHaveBeenCalledOnce(); expect(consume).toHaveBeenCalledOnce();
});
it.each(sides)("%s scope rejection leaves the retrieval trace and both frozen-record stores untouched", async side => {
  const f = await fixture(side), retrieval = {traceId: crypto.randomUUID(), query: "original query"}, request = {...f.request, retrieval};
  const commit = vi.fn((accept: () => void) => accept());
  f.runtime.execution.retrievalPreparation = async () => ({context: [{sourceId: `retrieval:${retrieval.traceId}`, revision: `trace:${retrieval.traceId}`, kind: "retrieval", resourceId: retrieval.traceId,
    anchor: JSON.stringify({manuscriptId: crypto.randomUUID(), traceId: retrieval.traceId}), text: "foreign traced context"}], commit});
  await expect(f.prepare(request)).rejects.toMatchObject({code: "invalidInput"}); expect(commit).not.toHaveBeenCalled();
  expect(f.runtime.execution.snapshot()).toHaveLength(0); expect(f.runtime.compaction.snapshot().records.size).toBe(0); expect((await f.status(request)).status.type).toBe("notFound");
});
it.each(sides)("%s status unavailability is readonly and does not consume the next prepare I/O fault", async side => {
  const f = await fixture(side), read = vi.spyOn(f.storage.files, "readFile"); f.fail("beforeCommit");
  if (side === "ordinary") f.runtime.execution.failPrepareStatus(true); else f.runtime.compaction.failPrepareStatus(true);
  expect((await f.status()).status).toMatchObject({type: "unconfirmed", detail: {operationId: f.request.requestId, safeCause: "mock_writing_prepare_reader_unavailable"}});
  await expect(f.prepare()).rejects.toBeDefined(); expect(read).not.toHaveBeenCalled();
});
it.each(sides)("%s resource I/O failure cannot silently publish a typed partial preparation", async side => {
  const f = await fixture(side), cause = new Error("synthetic resource port unavailable"), request = {...f.request, resources: [{kind: "note" as const, resourceId: crypto.randomUUID()}]};
  f.runtime.execution.contextSources = async () => {throw cause;};
  await expect(f.prepare(request)).rejects.toBe(cause); expect((await f.status(request)).status.type).toBe("notFound");
  expect(f.runtime.execution.snapshot()).toHaveLength(0); expect(f.runtime.compaction.snapshot().records.size).toBe(0);
});
it("manual W corruption cannot be hidden by echoing the queried revision", async () => {
  const f = await fixture("manual"); await f.prepare(); const snapshot = f.runtime.compaction.snapshot();
  for (const [key, value] of snapshot.records) snapshot.records.set(key, {...value, prepared: {...value.prepared, expectedRevision: 99}});
  f.runtime.compaction.restore(snapshot); expect((await f.status()).status.type).toBe("unconfirmed"); await expect(f.prepare()).rejects.toMatchObject({code: "recoveryRequired"});
});
