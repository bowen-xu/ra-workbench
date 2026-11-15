import {expect, it, vi} from "vitest";
import type {AgentSubmissionGateway} from "@research-assistant/data/agent";
import type {AgentSubmissionPreview, AgentWritingPrepareRequest, AgentWritingStartResponse} from "@research-assistant/core/agent";
import {ConversationDraftStore, conversationDraftKey} from "./conversationDraftStore";
import {localConversationDraftPersistence} from "./conversationDraftPersistence";
import {writingDraftAcknowledgments} from "./writingDraftAcknowledgment";
import {WritingSubmissionController} from "./writingSubmissionController";
import {registerStoragePersistence} from "../../runtime/storagePersistence";

const c = "11111111-1111-4111-8111-111111111111", m = "22222222-2222-4222-8222-222222222222";
const r = "33333333-3333-4333-8333-333333333333", t = "44444444-4444-4444-8444-444444444444";
const scope = {conversationId: c, manuscriptId: m}, draftScope = {hostKind: "manuscript" as const, hostId: m, conversationId: c};
const draftKey = conversationDraftKey(draftScope), locator = `research-assistant:writing-prepare:v1:${m}:${c}`;
const request: AgentWritingPrepareRequest = {conversationId: c, requestId: r, text: "original", context: {manuscriptId: m, files: []}};
const preview: AgentSubmissionPreview = {schemaVersion: 1, ...scope, requestId: r, turnId: t, contextSnapshotId: t,
  input: {text: "original", context: [{sourceId: "file", kind: "manuscript", resourceId: "main.tex", revision: "disk", anchor: JSON.stringify({manuscriptId: m}), text: "original source"}],
    model: {providerId: "fixture", modelId: "original", revision: null}, reasoningEffort: null}};
const admitted: AgentWritingStartResponse = {schemaVersion: 1, conversationId: c, requestId: r, turnId: t, alreadyStarted: false, outcome: null};
function fixture() {
  const rows = new Map<string, string>(); let failDraft = false;
  const storage: Storage = {get length() {return rows.size;}, key: index => [...rows.keys()][index] ?? null, getItem: key => rows.get(key) ?? null,
    setItem: (key, value) => {if (failDraft && key === "research-assistant.agent-drafts.v1") throw new Error("quota"); rows.set(key, value);},
    removeItem: key => {rows.delete(key);}, clear: () => rows.clear()};
  const gateway: AgentSubmissionGateway = {prepareWriting: vi.fn(async () => preview), prepareWritingStatus: vi.fn(), restoreWriting: vi.fn(),
    startWriting: vi.fn(async () => admitted), startStatus: vi.fn<AgentSubmissionGateway["startStatus"]>(async intent => ({schemaVersion: 1, intent, status: {type: "started", result: {...admitted, alreadyStarted: true, outcome: "completed"}}}))};
  const store = () => new ConversationDraftStore(localConversationDraftPersistence(storage));
  const open = (drafts: ConversationDraftStore) => {
    const owner = new WritingSubmissionController(gateway, {interrupt: vi.fn()}, undefined, {storage, scope});
    owner.configureDraftAcknowledgments(writingDraftAcknowledgments(drafts)); return owner;
  };
  const drafts = store(); drafts.update(draftKey, request.text);
  return {rows, storage, gateway, store, drafts, open, failDraft: (value: boolean) => {failDraft = value;}};
}
function persistence(storage: Storage, flush: () => Promise<void>) {
  registerStoragePersistence(storage, {backend: "IndexedDB", pending: false, error: undefined, resetting: false, flush,
    retry: async () => {}, reset: async () => {}, subscribe: () => () => {}, close: () => {}});
}
async function prepare(f: ReturnType<typeof fixture>, owner = f.open(f.drafts)) {
  const token = f.drafts.captureAcknowledgmentToken(draftKey, request.text)!;
  await owner.prepare(request, f.drafts.captureAcknowledgment(draftKey, request.text), [], token);
  await owner.observeWorkspacePreparation(r, true); return {owner, token};
}
it("flushes the complete draft and locator before prepare, retains both through workspace, and signs only after actual start", async () => {
  const f = fixture(), gates: (() => void)[] = []; persistence(f.storage, () => new Promise<void>(resolve => {gates.push(resolve);}));
  const owner = f.open(f.drafts), token = f.drafts.captureAcknowledgmentToken(draftKey, request.text)!;
  const work = owner.prepare(request, vi.fn(), [], token);
  await vi.waitFor(() => expect(gates).toHaveLength(1)); expect(f.gateway.prepareWriting).not.toHaveBeenCalled();
  expect(JSON.parse(f.rows.get(locator)!).draftAcknowledgment).toEqual(token);
  gates.shift()!(); await vi.waitFor(() => expect(gates).toHaveLength(1)); expect(f.gateway.prepareWriting).not.toHaveBeenCalled();
  gates.shift()!(); await vi.waitFor(() => expect(gates).toHaveLength(1)); gates.shift()!(); await work;
  expect(f.drafts.read(draftKey)).toBe("original"); persistence(f.storage, async () => {});
  await owner.observeWorkspacePreparation(r, true); expect(f.rows.has(locator)).toBe(true);
  expect(f.gateway.startWriting).not.toHaveBeenCalled(); await owner.confirm("chatGptDefault");
  expect(f.drafts.read(draftKey)).toBe(""); expect(f.rows.has(locator)).toBe(false);
  expect(f.gateway.startWriting).toHaveBeenCalledExactlyOnceWith({...scope, requestId: r, contextSnapshotId: t, model: preview.input.model, mode: "chatGptDefault"});
});
it.each(["unchanged", "sameText", "references"])("cold original start observation consumes only its exact draft version (%s)", async change => {
  const f = fixture(), {owner} = await prepare(f);
  vi.mocked(f.gateway.startWriting).mockRejectedValueOnce(new Error("lost ACK")); await owner.confirm("chatGptDefault"); owner.dispose();
  if (change === "sameText") {f.drafts.update(draftKey, ""); f.drafts.update(draftKey, request.text);}
  if (change === "references") f.drafts.updateStructured(draftKey, {...f.drafts.readStructured(draftKey), unboundReferences: [{type: "skill", name: "later", identity: {environment_id: "app", source_id: "later/SKILL.md", revision: "v2"}}]});
  const drafts = f.store(), restored = f.open(drafts), delivered = vi.fn();
  await restored.checkWritingPreparation(delivered);
  expect(restored.getSnapshot()).toMatchObject({phase: "started", outcome: "completed"});
  expect(drafts.read(draftKey)).toBe(change === "unchanged" ? "" : "original");
  if (change === "references") expect(drafts.readStructured(draftKey).unboundReferences).toHaveLength(1);
  expect(f.gateway.startWriting).toHaveBeenCalledOnce(); expect(f.gateway.prepareWriting).toHaveBeenCalledOnce();
  expect(f.gateway.prepareWritingStatus).not.toHaveBeenCalled(); expect(f.gateway.restoreWriting).not.toHaveBeenCalled();
  expect(delivered).toHaveBeenCalledOnce(); expect(f.rows.has(locator)).toBe(false);
});
it("keeps knownStarted durable before draft CAS and cold cleanup retries only local work", async () => {
  const f = fixture(), {owner} = await prepare(f);
  vi.mocked(f.gateway.startWriting).mockImplementationOnce(async () => {f.failDraft(true); return admitted;}); await owner.confirm("chatGptDefault");
  expect(owner.getSnapshot()).toMatchObject({phase: "started", writingPreparation: {phase: "localError", start: {result: admitted}}});
  expect(JSON.parse(f.rows.get(locator)!).start.result).toEqual(admitted); expect(f.drafts.read(draftKey)).toBe("original"); owner.dispose();
  const drafts = f.store(), restored = f.open(drafts); f.failDraft(false);
  await restored.retryWritingPreparation(); expect(drafts.read(draftKey)).toBe(""); expect(f.rows.has(locator)).toBe(false);
  expect(f.gateway.prepareWriting).toHaveBeenCalledOnce(); expect(f.gateway.startWriting).toHaveBeenCalledOnce();
  expect(f.gateway.startStatus).not.toHaveBeenCalled(); expect(f.gateway.prepareWritingStatus).not.toHaveBeenCalled();
});
it("a failed reliable knownStarted save cannot consume the draft or resend a model request", async () => {
  const f = fixture(), {owner} = await prepare(f), delivered = vi.fn(); let fail = true;
  persistence(f.storage, async () => {if (fail && JSON.parse(f.rows.get(locator) ?? "null")?.start?.result) throw new Error("disk");});
  await owner.confirm("chatGptDefault", delivered); expect(f.drafts.read(draftKey)).toBe("original"); expect(delivered).not.toHaveBeenCalled();
  expect(owner.getSnapshot().writingPreparation).toMatchObject({phase: "localError", start: {result: admitted}});
  fail = false; await owner.retryWritingPreparation(); expect(f.drafts.read(draftKey)).toBe("");
  expect(f.gateway.startWriting).toHaveBeenCalledOnce(); expect(f.gateway.startStatus).not.toHaveBeenCalled(); expect(delivered).toHaveBeenCalledOnce();
});
it("retries a failed locator deletion flush after its memory key is already gone without replaying CAS or Native", async () => {
  const f = fixture(), {owner} = await prepare(f); let fail = true;
  persistence(f.storage, async () => {if (fail && !f.rows.has(locator)) throw new Error("delete flush");});
  await owner.confirm("chatGptDefault"); expect(f.drafts.read(draftKey)).toBe(""); expect(f.rows.has(locator)).toBe(false);
  expect(owner.getSnapshot().writingPreparation).toMatchObject({phase: "localError", start: {result: admitted}});
  f.drafts.update(draftKey, "original"); fail = false; await owner.retryWritingPreparation();
  expect(owner.getSnapshot().writingPreparation?.phase).toBe("complete"); expect(f.drafts.read(draftKey)).toBe("original");
  expect(f.gateway.startWriting).toHaveBeenCalledOnce(); expect(f.gateway.startStatus).not.toHaveBeenCalled();
});
it("keeps a pending-conversation token through adoption and cold original-start observation", async () => {
  const f = fixture(), pending = conversationDraftKey({...draftScope, conversationId: null}); f.drafts.update(draftKey, ""); f.drafts.update(pending, "original");
  const token = f.drafts.captureAcknowledgmentToken(pending, "original")!; f.drafts.adoptPending(draftScope, c);
  const owner = f.open(f.drafts); await owner.prepare(request, undefined, [], token); await owner.observeWorkspacePreparation(r, true);
  vi.mocked(f.gateway.startWriting).mockRejectedValueOnce("lost ACK"); await owner.confirm("chatGptDefault"); owner.dispose();
  const drafts = f.store(), reopened = f.open(drafts); await reopened.checkWritingPreparation();
  expect(drafts.read(draftKey)).toBe(""); expect(drafts.read(pending)).toBe(""); expect(f.gateway.startWriting).toHaveBeenCalledOnce();
});
it("an old start ACK cannot overwrite a newer complete locator or consume its same-text draft", async () => {
  const f = fixture(), {owner} = await prepare(f); let resolve!: (value: AgentWritingStartResponse) => void;
  vi.mocked(f.gateway.startWriting).mockImplementationOnce(() => new Promise(done => {resolve = done;})); const work = owner.confirm("chatGptDefault");
  await vi.waitFor(() => expect(f.gateway.startWriting).toHaveBeenCalledOnce());
  f.drafts.update(draftKey, ""); f.drafts.update(draftKey, "original");
  const newer = JSON.stringify({...JSON.parse(f.rows.get(locator)!), intent: {...request, requestId: t}, start: null}); f.rows.set(locator, newer);
  resolve(admitted); await work; expect(f.rows.get(locator)).toBe(newer); expect(f.drafts.read(draftKey)).toBe("original");
  expect(owner.getSnapshot().writingPreparation?.phase).toBe("localError");
});
it("an old start ACK preserves a replacement same-R locator with another valid draft token and retries only local cleanup", async () => {
  const f = fixture(), {owner, token} = await prepare(f); let resolve!: (value: AgentWritingStartResponse) => void;
  vi.mocked(f.gateway.startWriting).mockImplementationOnce(() => new Promise(done => {resolve = done;}));
  const work = owner.confirm("chatGptDefault"); await vi.waitFor(() => expect(f.gateway.startWriting).toHaveBeenCalledOnce());
  f.drafts.update(draftKey, ""); f.drafts.update(draftKey, request.text);
  const replacement = f.drafts.captureAcknowledgmentToken(draftKey, request.text)!;
  expect(replacement.originKey).toBe(token.originKey); expect(replacement.revision).not.toBe(token.revision);
  const newer = JSON.stringify({...JSON.parse(f.rows.get(locator)!), draftAcknowledgment: replacement}); f.rows.set(locator, newer);
  resolve(admitted); await work;
  expect(f.rows.get(locator)).toBe(newer); expect(f.drafts.read(draftKey)).toBe(request.text);
  expect(owner.getSnapshot().writingPreparation).toMatchObject({phase: "localError", start: {result: admitted}});
  await owner.retryWritingPreparation();
  expect(f.rows.get(locator)).toBe(newer); expect(f.drafts.read(draftKey)).toBe(request.text);
  expect(owner.getSnapshot().writingPreparation).toMatchObject({phase: "localError", start: {result: admitted}});
  expect(f.gateway.prepareWriting).toHaveBeenCalledOnce(); expect(f.gateway.startWriting).toHaveBeenCalledOnce();
  expect(f.gateway.prepareWritingStatus).not.toHaveBeenCalled(); expect(f.gateway.startStatus).not.toHaveBeenCalled();
  expect(f.store().captureAcknowledgmentToken(draftKey, request.text)).toEqual(replacement);
});
it("completed original draft consumption cannot delete a replacement same-R token locator while ACK cleanup is awaiting its port", async () => {
  const f = fixture(), {owner, token} = await prepare(f), actual = writingDraftAcknowledgments(f.drafts);
  let release!: () => void, consumed = false;
  const pendingFlush = new Promise<void>(resolve => {release = resolve;});
  const consume = vi.fn(async (value: typeof token, accepted: typeof scope) => {
    await actual.consume(value, accepted); consumed = true; await pendingFlush;
  });
  owner.configureDraftAcknowledgments({...actual, consume});
  const work = owner.confirm("chatGptDefault"); await vi.waitFor(() => expect(consumed).toBe(true));
  expect(f.drafts.read(draftKey)).toBe(""); expect(f.store().read(draftKey)).toBe("");
  expect(JSON.parse(f.rows.get(locator)!).start.result).toEqual(admitted);
  f.drafts.update(draftKey, request.text);
  const replacement = f.drafts.captureAcknowledgmentToken(draftKey, request.text)!;
  expect(replacement.originKey).toBe(token.originKey); expect(replacement.revision).not.toBe(token.revision);
  const newer = JSON.stringify({...JSON.parse(f.rows.get(locator)!), draftAcknowledgment: replacement}); f.rows.set(locator, newer);
  release(); await work;
  expect(f.rows.get(locator)).toBe(newer); expect(f.drafts.read(draftKey)).toBe(request.text);
  expect(owner.getSnapshot().writingPreparation).toMatchObject({phase: "localError", start: {result: admitted}});
  await owner.retryWritingPreparation();
  expect(f.rows.get(locator)).toBe(newer); expect(f.drafts.read(draftKey)).toBe(request.text);
  expect(owner.getSnapshot().writingPreparation).toMatchObject({phase: "localError", start: {result: admitted}});
  expect(consume).toHaveBeenCalledExactlyOnceWith(token, scope);
  expect(f.store().captureAcknowledgmentToken(draftKey, request.text)).toEqual(replacement);
  expect(f.gateway.prepareWriting).toHaveBeenCalledOnce(); expect(f.gateway.startWriting).toHaveBeenCalledOnce();
  expect(f.gateway.prepareWritingStatus).not.toHaveBeenCalled(); expect(f.gateway.startStatus).not.toHaveBeenCalled();
});
