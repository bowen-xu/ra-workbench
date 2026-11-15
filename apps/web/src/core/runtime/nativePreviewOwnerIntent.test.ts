// @vitest-environment happy-dom
import {expect, it, vi} from "vitest";
import type {NativeAgentPreview} from "./nativeAgentPreview";
import {captureNewPreviewOwnerIntent, clearPreviewOwnerIntents, freezeNewPreviewOwnerIntent, readPreviewOwnerIntent, resolveWritingOwnerIntent, resolveOrRecoverPreviewOwnerIntent, NATIVE_PREVIEW_OWNER_CAPABILITY} from "./nativePreviewOwnerIntent";
import {registerStoragePersistence} from "./storagePersistence";
import {createTestNamespacedStorage} from "./testing/createTestNamespacedStorage";
import {aiAssistanceErrorMessage} from "../services/aiAssistanceErrorMessage";
import {diagnosticError} from "../services/diagnosticError";
const conversation = "11111111-1111-4111-8111-111111111111", request = "22222222-2222-4222-8222-222222222222";
const fixture = "33333333-3333-4333-8333-333333333333";

it("persists the original owner before returning, even if the active owner changes", async () => {
  const storage = createTestNamespacedStorage(window.localStorage, `owner-intent-${crypto.randomUUID()}:`);
  storage.clear();
  let finish!: () => void;
  const flush = vi.fn(() => new Promise<void>(resolve => {finish = resolve;}));
  registerStoragePersistence(storage, {backend:"IndexedDB", pending:false, error:undefined, resetting:false,
    flush, retry:async()=>{}, reset:async()=>{}, subscribe:()=>()=>{}, close:()=>{}});
  let fixtureId: string | null = fixture;
  const preview = {bootstrap:{generation:"generation-a",supported:[NATIVE_PREVIEW_OWNER_CAPABILITY]},
    snapshotConversationOwner:()=>({generation:"generation-a",fixtureId})} as unknown as NativeAgentPreview;
  const pending = freezeNewPreviewOwnerIntent(preview, storage, conversation, request);
  fixtureId = null;
  expect(readPreviewOwnerIntent(storage, conversation, request)).toEqual({fixtureId:fixture});
  expect(flush).toHaveBeenCalledOnce();
  finish();
  await pending;
  await expect(freezeNewPreviewOwnerIntent(preview, storage, conversation, request)).rejects.toThrow("agent_preview_owner_intent_conflict");
});

it("rejects a reset racing with persistence and leaves unsupported runtimes untouched", async () => {
  const storage = createTestNamespacedStorage(window.localStorage, `owner-intent-${crypto.randomUUID()}:`);
  storage.clear();
  const bootstrap = {generation:"before",supported:[NATIVE_PREVIEW_OWNER_CAPABILITY]};
  registerStoragePersistence(storage, {backend:"IndexedDB", pending:false, error:undefined, resetting:false,
    flush:async()=>{bootstrap.generation="after";}, retry:async()=>{}, reset:async()=>{}, subscribe:()=>()=>{}, close:()=>{}});
  const preview = {bootstrap,snapshotConversationOwner:()=>({generation:"before",fixtureId:null})} as unknown as NativeAgentPreview;
  await expect(freezeNewPreviewOwnerIntent(preview, storage, conversation, request)).rejects.toThrow("agent_preview_stale_owner");
  bootstrap.supported=[];
  await freezeNewPreviewOwnerIntent(preview, storage, conversation, fixture);
  expect(()=>readPreviewOwnerIntent(storage, conversation, fixture)).toThrow("agent_preview_owner_intent_missing");
});

it("reads the frozen owner from a reopened storage view and rejects missing or corrupt intent", async () => {
  const prefix = `owner-intent-${crypto.randomUUID()}:`;
  const storage = createTestNamespacedStorage(window.localStorage, prefix);
  registerStoragePersistence(storage, {backend:"IndexedDB", pending:false, error:undefined, resetting:false,
    flush:async()=>{}, retry:async()=>{}, reset:async()=>{}, subscribe:()=>()=>{}, close:()=>{}});
  const preview = {bootstrap:{generation:"g",supported:[NATIVE_PREVIEW_OWNER_CAPABILITY]},
    snapshotConversationOwner:()=>({generation:"g",fixtureId:fixture})} as unknown as NativeAgentPreview;
  await freezeNewPreviewOwnerIntent(preview, storage, conversation, request);
  const reopened = createTestNamespacedStorage(window.localStorage, prefix);
  expect(readPreviewOwnerIntent(reopened, conversation, request)).toEqual({fixtureId:fixture});
  reopened.setItem(`research-assistant:preview-owner-intent:v1:${conversation}:${request}`, "{}");
  expect(()=>readPreviewOwnerIntent(reopened, conversation, request)).toThrow("agent_preview_owner_intent_invalid");
  reopened.clear();
  expect(()=>readPreviewOwnerIntent(reopened, conversation, request)).toThrow("agent_preview_owner_intent_missing");
});

it("resolves ordinary and queued retries using the same previously persisted owner", async () => {
  const prefix = `owner-intent-${crypto.randomUUID()}:`;
  const storage = createTestNamespacedStorage(window.localStorage, prefix);
  registerStoragePersistence(storage, {backend:"IndexedDB", pending:false, error:undefined, resetting:false,
    flush:async()=>{}, retry:async()=>{}, reset:async()=>{}, subscribe:()=>()=>{}, close:()=>{}});
  const preview = {bootstrap:{generation:"g",supported:[NATIVE_PREVIEW_OWNER_CAPABILITY]},
    snapshotConversationOwner:()=>({generation:"g",fixtureId:fixture})} as unknown as NativeAgentPreview;
  await freezeNewPreviewOwnerIntent(preview, storage, conversation, request);
  const reopened = createTestNamespacedStorage(window.localStorage, prefix);
  const submission = {conversationId:conversation,requestId:request};
  for (const [command, data] of [
    ["agent_prepare_writing_submission", submission], ["agent_start_writing_submission", submission],
    ["agent_prepare_writing_queue_preview", {submission}], ["agent_start_writing_queued_submission", {start:submission}],
    ["agent_prepare_writing_compaction", submission], ["agent_start_compaction", submission],
    ["agent_prepare_writing_queued_compaction", {submission}], ["agent_settle_writing_queued_compaction", submission],
    ["agent_cancel_writing_queue_preview", {conversationId:conversation, previewRequestId:request, operationId:fixture}],
  ] as const) expect(resolveWritingOwnerIntent(reopened, command, {request:data})).toEqual({conversationId:conversation,fixtureId:fixture});
  for (const command of ["agent_edit_writing_input_queue", "agent_edit_context_input_queue", "agent_edit_note_input_queue", "agent_edit_paper_input_queue"])
    expect(resolveWritingOwnerIntent(reopened, command, {request:submission})).toEqual({conversationId:conversation,fixtureId:fixture});
  for (const command of ["agent_steer_writing_turn", "agent_steer_context_turn", "agent_steer_note_turn", "agent_steer_paper_turn"])
    expect(resolveWritingOwnerIntent(reopened, command, {request:{conversationId:conversation,inputId:request}})).toEqual({conversationId:conversation,fixtureId:fixture});
  expect(resolveWritingOwnerIntent(reopened,"agent_prepare_steering_handoff", {request:{conversationId:conversation,operationId:request}}))
    .toEqual({conversationId:conversation,fixtureId:fixture});
  for (const command of ["agent_finish_steering_handoff", "agent_release_steering_handoff", "agent_cancel_unregistered_steering_handoff"])
    expect(resolveWritingOwnerIntent(reopened,command,{request:{intent:{conversationId:conversation,operationId:request},expectedEpoch:"epoch"}}))
      .toEqual({conversationId:conversation,fixtureId:fixture});
  expect(()=>resolveWritingOwnerIntent(reopened,"agent_prepare_writing_submission", {request:{conversationId:conversation,requestId:fixture}})).toThrow("agent_preview_owner_intent_missing");
  expect(resolveWritingOwnerIntent(reopened,"agent_read_history", {request:submission})).toBeUndefined();
  reopened.clear();
});

it("recovers only a matching native proof before replay, even after the active scenario changes", async () => {
  const prefix=`owner-recovery-${crypto.randomUUID()}:`, storage=createTestNamespacedStorage(window.localStorage,prefix);
  let finish!:()=>void;
  const flush=vi.fn(()=>new Promise<void>(resolve=>{finish=resolve;}));
  registerStoragePersistence(storage,{backend:"IndexedDB",pending:false,error:undefined,resetting:false,flush,retry:async()=>{},reset:async()=>{},subscribe:()=>()=>{},close:()=>{}});
  const invoke=vi.fn(async()=>({schemaVersion:1,status:"found",generation:"g",conversationId:conversation,requestId:request,fixtureId:fixture}));
  const preview={bootstrap:{generation:"g",supported:[NATIVE_PREVIEW_OWNER_CAPABILITY]},invoke,
    snapshotConversationOwner:()=>({generation:"g",fixtureId:null})} as unknown as NativeAgentPreview;
  const pending=resolveOrRecoverPreviewOwnerIntent(preview,storage,"agent_prepare_writing_submission",{request:{conversationId:conversation,requestId:request}});
  await vi.waitFor(()=>expect(flush).toHaveBeenCalledOnce());
  expect(invoke).toHaveBeenCalledExactlyOnceWith("agent_preview_read_owner_proof",{request:{conversationId:conversation,requestId:request,expectedGeneration:"g"}});
  expect(readPreviewOwnerIntent(storage,conversation,request)).toEqual({fixtureId:fixture});
  let settled=false; void pending.then(()=>{settled=true;}); await Promise.resolve(); expect(settled).toBe(false);
  finish(); expect(await pending).toEqual({conversationId:conversation,fixtureId:fixture});
  const reopened=createTestNamespacedStorage(window.localStorage,prefix);
  expect(readPreviewOwnerIntent(reopened,conversation,request)).toEqual({fixtureId:fixture});
  reopened.clear();
});

it("does not infer an owner when native proof is missing or belongs to another identity", async () => {
  const storage=createTestNamespacedStorage(window.localStorage,`owner-missing-${crypto.randomUUID()}:`);
  registerStoragePersistence(storage,{backend:"IndexedDB",pending:false,error:undefined,resetting:false,flush:async()=>{},retry:async()=>{},reset:async()=>{},subscribe:()=>()=>{},close:()=>{}});
  const invoke=vi.fn(async()=>({schemaVersion:1,status:"missing",generation:"g",conversationId:conversation,requestId:request}));
  const preview={bootstrap:{generation:"g",supported:[NATIVE_PREVIEW_OWNER_CAPABILITY]},invoke} as unknown as NativeAgentPreview;
  const args={request:{conversationId:conversation,requestId:request}};
  const missing = await resolveOrRecoverPreviewOwnerIntent(preview,storage,"agent_prepare_writing_submission",args).catch(cause=>cause as unknown);
  expect(missing).toBeInstanceOf(Error);
  expect(diagnosticError(missing).code).toBe("AGENT_PREVIEW_OWNER_INTENT_MISSING");
  expect(aiAssistanceErrorMessage(missing)).toContain("原请求的归属尚无法确认");
  expect(()=>readPreviewOwnerIntent(storage,conversation,request)).toThrow("agent_preview_owner_intent_missing");
  invoke.mockResolvedValueOnce({schemaVersion:1,status:"found",generation:"g",conversationId:fixture,requestId:request,fixtureId:null} as never);
  await expect(resolveOrRecoverPreviewOwnerIntent(preview,storage,"agent_prepare_writing_submission",args)).rejects.toThrow("agent_preview_owner_proof_invalid");
  storage.clear();
});

it("does not dispatch from an in-memory recovered proof after its persistence fails", async () => {
  const storage=createTestNamespacedStorage(window.localStorage,`owner-flush-${crypto.randomUUID()}:`);
  const flush=vi.fn().mockRejectedValue(new Error("quota"));
  registerStoragePersistence(storage,{backend:"IndexedDB",pending:false,error:undefined,resetting:false,flush,retry:async()=>{},reset:async()=>{},subscribe:()=>()=>{},close:()=>{}});
  const invoke=vi.fn(async()=>({schemaVersion:1,status:"found",generation:"g",conversationId:conversation,requestId:request,fixtureId:fixture}));
  const preview={bootstrap:{generation:"g",supported:[NATIVE_PREVIEW_OWNER_CAPABILITY]},invoke} as unknown as NativeAgentPreview;
  const args={request:{conversationId:conversation,requestId:request}};
  await expect(resolveOrRecoverPreviewOwnerIntent(preview,storage,"agent_prepare_writing_submission",args)).rejects.toThrow("quota");
  await expect(resolveOrRecoverPreviewOwnerIntent(preview,storage,"agent_prepare_writing_submission",args)).rejects.toThrow("quota");
  expect(invoke).toHaveBeenCalledOnce(); expect(flush).toHaveBeenCalledTimes(2);
  storage.clear();
});

it("clears only owner intents after confirmed reset, and retries an interrupted flush", async () => {
  const storage = createTestNamespacedStorage(window.localStorage, `owner-intent-${crypto.randomUUID()}:`);
  const failure = new Error("idb failure"); let error: Error | undefined;
  const retry = vi.fn(async()=>{error=undefined;});
  const flush = vi.fn(async()=>{if (!error) {error=failure; throw failure;}});
  registerStoragePersistence(storage, {backend:"IndexedDB",pending:false,get error(){return error;},resetting:false,
    flush,retry,reset:async()=>{},subscribe:()=>()=>{},close:()=>{}});
  storage.setItem(`research-assistant:preview-owner-intent:v1:${conversation}:${request}`,"saved");
  storage.setItem("other-draft","preserved");
  await expect(clearPreviewOwnerIntents(storage)).rejects.toBe(failure);
  flush.mockImplementation(async()=>{});
  await clearPreviewOwnerIntents(storage);
  expect(retry).toHaveBeenCalledOnce();
  expect(storage.getItem(`research-assistant:preview-owner-intent:v1:${conversation}:${request}`)).toBeNull();
  expect(storage.getItem("other-draft")).toBe("preserved");
  storage.clear();
});

it("uses the initially captured owner for a rebased or retried request", async () => {
  const storage = createTestNamespacedStorage(window.localStorage, `owner-intent-${crypto.randomUUID()}:`);
  let failure: Error | undefined;
  const flush = vi.fn(async()=>{if (!failure) {failure=new Error("transient");throw failure;}});
  const retry = vi.fn(async()=>{failure=undefined;flush.mockImplementation(async()=>{});});
  registerStoragePersistence(storage,{backend:"IndexedDB",pending:false,get error(){return failure;},resetting:false,
    flush,retry,reset:async()=>{},subscribe:()=>()=>{},close:()=>{}});
  let active: string | null = fixture;
  const preview = {bootstrap:{generation:"g",supported:[NATIVE_PREVIEW_OWNER_CAPABILITY]},
    snapshotConversationOwner:()=>({generation:"g",fixtureId:active})} as unknown as NativeAgentPreview;
  const persist = captureNewPreviewOwnerIntent(preview,storage,conversation)!;
  active=null;
  await expect(persist(request)).rejects.toThrow("transient");
  await persist(request);
  const rebased = "44444444-4444-4444-8444-444444444444";
  await persist(rebased);
  expect(readPreviewOwnerIntent(storage,conversation,request)).toEqual({fixtureId:fixture});
  expect(readPreviewOwnerIntent(storage,conversation,rebased)).toEqual({fixtureId:fixture});
  storage.clear();
});
