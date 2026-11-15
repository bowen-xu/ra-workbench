import {expect, it, vi} from "vitest";
import type {AgentSubmissionGateway} from "@research-assistant/data/agent";
import {decodeAgentWritingPrepareRequest, type AgentSubmissionPreview, type AgentWritingPrepareRequest} from "@research-assistant/core/agent";
import {registerStoragePersistence} from "../../runtime/storagePersistence";
import {WritingPreparation} from "./writingPreparation";
import {writingPreparationMessage} from "./writingPreparationMessage";
const c = "11111111-1111-4111-8111-111111111111", m = "22222222-2222-4222-8222-222222222222", r = "33333333-3333-4333-8333-333333333333";
const scope = {conversationId: c, manuscriptId: m}, revision = "44444444-4444-4444-8444-444444444444";
const request: AgentWritingPrepareRequest = decodeAgentWritingPrepareRequest({conversationId: c, requestId: r, text: "original input", model: {providerId: "fixture", modelId: "original"},
  retrieval: {traceId: "55555555-5555-4555-8555-555555555555", query: "original query"}, context: {manuscriptId: m, files: [{relativePath: "main.tex", expectedDiskSha256: "a".repeat(64), buffer: {revision, text: "original unsaved"}}]}});
const buffers = [{path: "main.tex", sessionId: revision, revision, expectedDiskSha256: "a".repeat(64), text: "original unsaved"}];
const rollback = {code: "storage", reason: {conversationId: c, manuscriptId: m, requestId: r, operationId: r, code: "DATA_SQL_OPERATION_CONFLICT",
  reason: {operationId: r, epoch: null, sqlFence: null, phase: "writing-prepare-materialize", sqliteCode: null, sqliteExtendedCode: null, safeCause: "writing_context_changed"}}};
const unavailableBinding = {...rollback, reason: {...rollback.reason, code: "DATA_SQL_INVALID", reason: {...rollback.reason.reason, safeCause: "writing_binding_unavailable"}}};
const preview: AgentSubmissionPreview = {schemaVersion: 1, ...scope, requestId: r, turnId: revision, contextSnapshotId: revision,
  input: {text: request.text, model: {providerId: "fixture", modelId: "original", revision: null}, reasoningEffort: null,
    context: [{sourceId: "file", kind: "manuscript", resourceId: "main.tex", revision: "disk", anchor: JSON.stringify({manuscriptId: m}), text: "original unsaved"}]}};
function fixture() {
  const rows = new Map<string, string>(), storage: Storage = {get length() {return rows.size;}, key: index => [...rows.keys()][index] ?? null,
    getItem: key => rows.get(key) ?? null, setItem: (key, value) => {rows.set(key, value);}, removeItem: key => {rows.delete(key);}, clear: () => rows.clear()};
  const prepare = vi.fn<AgentSubmissionGateway["prepareWriting"]>().mockRejectedValue("AGENT_PREVIEW_SYNTHETIC_IO_ACK_LOST");
  const status = vi.fn<AgentSubmissionGateway["prepareWritingStatus"]>().mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "notFound"}}));
  const gateway: AgentSubmissionGateway = {prepareWriting: prepare, prepareWritingStatus: status, restoreWriting: vi.fn(), startWriting: vi.fn(), startStatus: vi.fn()};
  const open = (isAdmissionDisabled?: () => boolean) => new WritingPreparation(gateway, storage, () => {}, scope, isAdmissionDisabled);
  return {rows, storage, prepare, status, gateway, open};
}
function persistence(storage: Storage, flush: () => Promise<void>) {
  registerStoragePersistence(storage, {backend: "IndexedDB", pending: false, error: undefined, resetting: false, flush, retry: async () => {}, reset: async () => {}, subscribe: () => () => {}, close: () => {}});
}
it("retains the proven start until asynchronous context acceptance succeeds after reopening", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(preview);
  const started = {schemaVersion:1 as const,conversationId:c,requestId:r,turnId:revision,alreadyStarted:false,outcome:null};
  vi.mocked(f.gateway.startWriting).mockResolvedValue(started);
  await owner.begin(request,buffers); await owner.finishWorkspace(r);
  owner.configureStartObserver(async () => {throw new Error("context unconfirmed");});
  await owner.confirmStart({...scope,requestId:r,contextSnapshotId:revision,model:preview.input.model,mode:"chatGptDefault"});
  expect(owner.getSnapshot()).toMatchObject({phase:"localError",start:{result:started}});
  expect(JSON.parse([...f.rows.values()][0]!).start.result).toEqual(started);
  owner.dispose(); const reopened=f.open(), accepted=vi.fn(async()=>{}); reopened.configureStartObserver(accepted);
  await reopened.retry(); expect(accepted).toHaveBeenCalledOnce(); expect(reopened.getSnapshot().phase).toBe("complete");
  expect(f.rows.size).toBe(0); expect(f.gateway.startWriting).toHaveBeenCalledOnce();
});
it("reliably saves the full request and original workspace buffers before the first I/O", async () => {
  const f = fixture(), owner = f.open(), caller = structuredClone(request); let release!: () => void;
  persistence(f.storage, () => new Promise<void>(resolve => {release = resolve;})); const work = owner.begin(caller, buffers);
  await vi.waitFor(() => expect(f.rows.size).toBe(1)); expect(f.prepare).not.toHaveBeenCalled();
  (caller as {text: string}).text = "later"; expect(JSON.parse([...f.rows.values()][0]!).intent).toEqual(request);
  release(); await work; expect(f.prepare).toHaveBeenCalledExactlyOnceWith(request); expect(owner.getSnapshot().workspaceBuffers).toEqual(buffers);
});
it("reopens Unknown read-only and retries the same full intent after NotFound, preserving buffers until workspace confirmation", async () => {
  const f = fixture(), owner = f.open(); await owner.begin(request, buffers); owner.dispose(); const restored = f.open();
  expect(f.status).not.toHaveBeenCalled(); expect(restored.getSnapshot()).toMatchObject({phase: "unknown", intent: request, priorUnknown: true, workspaceBuffers: buffers});
  await restored.check(); expect(f.prepare).toHaveBeenCalledOnce(); expect(restored.getSnapshot().phase).toBe("notFound");
  await restored.retire(); expect(f.rows.size).toBe(1); f.prepare.mockResolvedValue(preview); await restored.retry();
  expect(f.prepare.mock.calls.map(([value]) => value)).toEqual([request, request]); expect(restored.getSnapshot().phase).toBe("ready"); expect(f.rows.size).toBe(1);
  await restored.finishWorkspace(r); expect(f.rows.size).toBe(1); expect(restored.getSnapshot().phase).toBe("ready");
  await restored.finishWorkspace(r); await restored.check(); await restored.retry();
  expect(restored.getSnapshot().phase).toBe("ready"); expect(f.rows.size).toBe(1); expect(f.prepare).toHaveBeenCalledTimes(2); expect(f.status).toHaveBeenCalledTimes(2);
});
it("a committed status recovers the original preview without rematerializing resources, files, model or retrieval", async () => {
  const f = fixture(), owner = f.open(); await owner.begin(request, buffers); owner.dispose(); const restored = f.open();
  f.status.mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "committed", result: preview,
    proof: {schemaVersion: 1, operationId: r, conversationId: c, family: "agent.writing.submission.prepare", epoch: revision, sqlFence: "0", replayed: true, result: null}}}));
  await restored.check(); expect(restored.getSnapshot()).toMatchObject({phase: "ready", result: preview, knownCommitted: true});
  expect(f.prepare).toHaveBeenCalledOnce(); expect(f.status).toHaveBeenCalledExactlyOnceWith(request); expect(f.gateway.startWriting).not.toHaveBeenCalled();
});
it("knownCommitted local failures only retry local persistence and removal, including after reopening", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockResolvedValue(preview); let writes = 0;
  persistence(f.storage, async () => {if (++writes === 2) throw new Error("local persistence failed");});
  await owner.begin(request, buffers); expect(owner.getSnapshot()).toMatchObject({phase: "localError", knownCommitted: true}); owner.dispose();
  const restored = f.open(); await restored.retry(); expect(restored.getSnapshot().phase).toBe("ready"); expect(f.prepare).toHaveBeenCalledOnce(); expect(f.status).not.toHaveBeenCalled();
  let fail = true; persistence(f.storage, async () => {if (fail) throw new Error("local cleanup failed");});
  await restored.finishWorkspace(r); expect(restored.getSnapshot()).toMatchObject({phase: "localError", workspaceReady: true});
  fail = false; await restored.retry(); expect(restored.getSnapshot().phase).toBe("ready"); expect(f.prepare).toHaveBeenCalledOnce(); expect(f.status).not.toHaveBeenCalled();
});
it.each([rollback, unavailableBinding])("prior Unknown cannot be retired after a later explicit prewrite rejection or readonly NotFound (%j)", async cause => {
  const f = fixture(), owner = f.open(); await owner.begin(request, buffers); f.prepare.mockRejectedValue(cause);
  await owner.retry(); expect(owner.getSnapshot()).toMatchObject({phase: "unknown", priorUnknown: true, knownRejected: false});
  owner.dispose(); const restored = f.open(); await restored.check(); await restored.retire();
  expect(f.rows.size).toBe(1); expect(restored.getSnapshot()).toMatchObject({phase: "notFound", knownRejected: false, priorUnknown: true});
});
it.each(["modelUnavailable", rollback, unavailableBinding])("a first known rejection remains explicit after reopening and only NotFound allows returning to editing (%j)", async cause => {
  const f = fixture(), owner = f.open(); f.prepare.mockRejectedValue(cause); await owner.begin(request, buffers);
  expect(owner.getSnapshot()).toMatchObject({knownRejected: true, priorUnknown: false}); owner.dispose(); const restored = f.open();
  expect(restored.getSnapshot().cause).toEqual(cause);
  await restored.retire(); expect(f.status).toHaveBeenCalledExactlyOnceWith(request); expect(f.prepare).toHaveBeenCalledOnce(); expect(f.rows.size).toBe(0);
  expect(restored.getSnapshot().phase).toBe("retired");
});
it("keeps a deleted historical preparation Unconfirmed and never treats its missing result as permission to retire", async () => {
  const f = fixture(), owner = f.open(); await owner.begin(request, buffers); owner.dispose(); const restored = f.open();
  const detail = {...rollback.reason.reason, phase: "prepare-status", safeCause: "writing_prepare_result_deleted"};
  f.status.mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "unconfirmed", detail}}));
  await restored.check(); await restored.retire();
  expect(restored.getSnapshot()).toMatchObject({phase: "unconfirmed", priorUnknown: true, knownRejected: false, status: {type: "unconfirmed", detail}});
  expect(f.rows.size).toBe(1); expect(f.prepare).toHaveBeenCalledOnce(); expect(f.gateway.startWriting).not.toHaveBeenCalled();
});
it("disposal during reliable save prevents the first RPC and late ACK cannot replace a newer full locator", async () => {
  const f = fixture(), owner = f.open(); let release!: () => void;
  persistence(f.storage, () => new Promise<void>(resolve => {release = resolve;})); const work = owner.begin(request, buffers);
  await vi.waitFor(() => expect(release).toBeTypeOf("function")); owner.dispose(); release(); await work; expect(f.prepare).not.toHaveBeenCalled();
  const g = fixture(), other = g.open(); let ack!: (value: AgentSubmissionPreview) => void;
  g.prepare.mockImplementation(() => new Promise(resolve => {ack = resolve;})); const pending = other.begin(request, buffers);
  await vi.waitFor(() => expect(g.prepare).toHaveBeenCalledOnce()); other.dispose(); const locator = [...g.rows.keys()][0]!;
  const newer = JSON.stringify({...JSON.parse(g.rows.get(locator)!), intent: {...request, requestId: revision}}); g.rows.set(locator, newer); ack(preview); await pending;
  expect(g.rows.get(locator)).toBe(newer);
});

it.each(["receipt", "draft"] as const)("blocks the first ordinary prepare after the %s flush without inventing Native rejection", async boundary => {
  const f = fixture(); let disabled = false, release!: () => void;
  const owner = f.open(() => disabled), gate = new Promise<void>(resolve => {release = resolve;});
  const draftFlush = vi.fn(() => gate);
  if (boundary === "receipt") persistence(f.storage, () => gate);
  else owner.configureDraftAcknowledgments({flush: draftFlush, retry: async () => {}, consume: async () => {throw new Error("unexpected acknowledgment");}});
  const token = boundary === "draft" ? {schemaVersion: 1 as const, originKey: JSON.stringify(["manuscript", m, c]), revision} : null;
  const work = owner.begin(request, buffers, token);
  await vi.waitFor(() => expect(owner.getSnapshot().intent).toEqual(request));
  if (boundary === "draft") await vi.waitFor(() => expect(draftFlush).toHaveBeenCalledOnce());
  disabled = true; release(); await work;
  expect(f.prepare).not.toHaveBeenCalled();
  expect(owner.getSnapshot()).toMatchObject({phase: "localError", priorUnknown: false, knownRejected: false, cause: {code: "WRITING_DISPATCH_ADMISSION_CLOSED", step: "prepare"}});
  expect(writingPreparationMessage(owner.getSnapshot())).toContain("此步骤尚未派发");
  expect(writingPreparationMessage(owner.getSnapshot())).toContain(`R=${r}`);
  await owner.check(); await owner.retireVerifiedUnstarted();
  expect(owner.getSnapshot().phase).toBe("retired"); expect(f.rows.size).toBe(0);
});

it("prepare having dispatched never authorizes a fresh start after reliable saving closes the instance", async () => {
  const f = fixture(); let disabled = false, release!: () => void;
  const owner = f.open(() => disabled); f.prepare.mockResolvedValue(preview);
  await owner.begin(request, buffers); await owner.finishWorkspace(r);
  persistence(f.storage, () => new Promise<void>(resolve => {release = resolve;}));
  const start = {...scope, requestId: r, contextSnapshotId: revision, model: preview.input.model, mode: "chatGptDefault" as const};
  const work = owner.confirmStart(start); await vi.waitFor(() => expect(release).toBeTypeOf("function"));
  disabled = true; release(); await work;
  expect(f.prepare).toHaveBeenCalledOnce(); expect(f.gateway.startWriting).not.toHaveBeenCalled();
  expect(owner.getSnapshot().start).toMatchObject({request: start, priorUnknown: false, knownRejected: false});
});

it("allows the exact already-dispatched Unknown prepare retry and preserves the closed-instance ACK", async () => {
  const f = fixture(); let disabled = false; const owner = f.open(() => disabled);
  await owner.begin(request, buffers); disabled = true; f.prepare.mockResolvedValue(preview);
  await owner.retry(); expect(f.prepare.mock.calls.map(([value]) => value)).toEqual([request, request]);
  expect(owner.getSnapshot()).toMatchObject({knownCommitted: true, result: preview});
  owner.dispose(); const reopened = f.open(() => true); await reopened.check();
  expect(f.prepare).toHaveBeenCalledTimes(2); expect(reopened.getSnapshot().knownCommitted).toBe(true);
});

it.each(["prepare", "start"] as const)("checks %s admission after synchronous sending subscribers", async step => {
  const f = fixture(); let disabled = false;
  const owner = new WritingPreparation(f.gateway, f.storage, state => {
    if (state.phase === "sending" && (step === "prepare" || state.start !== null)) disabled = true;
  }, scope, () => disabled);
  f.prepare.mockResolvedValue(preview);
  await owner.begin(request, buffers);
  if (step === "prepare") expect(f.prepare).not.toHaveBeenCalled();
  else {
    await owner.finishWorkspace(r);
    await owner.confirmStart({...scope, requestId: r, contextSnapshotId: revision, model: preview.input.model, mode: "chatGptDefault"});
    expect(f.prepare).toHaveBeenCalledOnce(); expect(f.gateway.startWriting).not.toHaveBeenCalled();
  }
  expect(owner.getSnapshot().cause).toMatchObject({code: "WRITING_DISPATCH_ADMISSION_CLOSED", step});
});

it("treats a cold locally blocked prepare conservatively and keeps original same-intent recovery", async () => {
  const f = fixture(), owner = f.open(() => true); await owner.begin(request, buffers);
  expect(f.prepare).not.toHaveBeenCalled(); owner.dispose();
  const cold = f.open(() => true); await cold.check(); await cold.retireVerifiedUnstarted();
  expect(f.rows.size).toBe(1); expect(cold.getSnapshot().priorUnknown).toBe(true);
  f.prepare.mockResolvedValue(preview); await cold.retry();
  expect(f.prepare).toHaveBeenCalledExactlyOnceWith(request); expect(cold.getSnapshot().workspaceBuffers).toEqual(buffers);
});

it.each(["prepare", "start"] as const)("requires fresh admission after a definitive ordinary %s rejection", async step => {
  const f = fixture(); let disabled = false; const owner = f.open(() => disabled);
  if (step === "prepare") {
    f.prepare.mockRejectedValueOnce("modelUnavailable"); await owner.begin(request, buffers);
    expect(owner.getSnapshot()).toMatchObject({knownRejected: true, priorUnknown: false});
    disabled = true; f.prepare.mockResolvedValue(preview); await owner.retry();
    expect(f.prepare).toHaveBeenCalledOnce();
  } else {
    f.prepare.mockResolvedValue(preview); await owner.begin(request, buffers); await owner.finishWorkspace(r);
    vi.mocked(f.gateway.startWriting).mockRejectedValueOnce("modelUnavailable");
    vi.mocked(f.gateway.startStatus).mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "prepared", preview}}));
    const start = {...scope, requestId: r, contextSnapshotId: revision, model: preview.input.model, mode: "chatGptDefault" as const};
    await owner.confirmStart(start); expect(owner.getSnapshot().start).toMatchObject({knownRejected: true, priorUnknown: false});
    disabled = true; await owner.confirmStart(start); expect(f.gateway.startWriting).toHaveBeenCalledOnce();
  }
  expect(owner.getSnapshot().cause).toMatchObject({code: "WRITING_DISPATCH_ADMISSION_CLOSED", step});
});

it("shows finite Native source identities through the production preparation feedback", async () => {
  const f = fixture();
  f.prepare.mockRejectedValue({...rollback, reason: {...rollback.reason,
    source: {kind: "native", packageKey: "a".repeat(64), instanceId: revision, requestId: revision, rejection: "conflict"}}});
  const owner = f.open();
  await owner.begin(request, buffers);
  const message = writingPreparationMessage(owner.getSnapshot());
  for (const token of ["writing_context_changed", `packageKey=${"a".repeat(64)}`, `nativeRequestId=${revision}`]) expect(message).toContain(token);
});
