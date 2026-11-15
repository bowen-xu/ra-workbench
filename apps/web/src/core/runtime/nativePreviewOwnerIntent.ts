import type { NativeAgentPreview } from "./nativeAgentPreview";
import { flushStorage, storagePersistence } from "./storagePersistence";

/** Activated only after the native preview advertises the complete protocol. */
export const NATIVE_PREVIEW_OWNER_CAPABILITY = "canvas-conversation-owner-v1";
const prefix = "research-assistant:preview-owner-intent:v1:";
const uuid = /^[\da-f]{8}-(?:[\da-f]{4}-){3}[\da-f]{12}$/iu;
const keyFor = (conversationId: string, requestId: string): string =>
  `${prefix}${conversationId.toLowerCase()}:${requestId.toLowerCase()}`;
class MissingPreviewOwnerIntent extends Error {
  readonly code = "AGENT_PREVIEW_OWNER_INTENT_MISSING";
  constructor() {super("agent_preview_owner_intent_missing");}
}

/** Called only after the original native reset has returned a confirmed receipt. */
export async function clearPreviewOwnerIntents(storage: Storage): Promise<void> {
  const persistence = storagePersistence(storage);
  if (persistence?.error) await persistence.retry();
  const keys: string[] = [];
  for (let index = 0; index < storage.length; index++) {
    const key = storage.key(index);
    if (key?.startsWith(prefix)) keys.push(key);
  }
  for (const key of keys) storage.removeItem(key);
  await flushStorage(storage);
}

/** Missing legacy intent is unknown, not evidence of an ordinary owner. */
export function readPreviewOwnerIntent(storage: Storage, conversationId: string, requestId: string): {fixtureId: string | null} {
  if (!uuid.test(conversationId) || !uuid.test(requestId)) throw new Error("agent_preview_owner_invalid");
  const raw = storage.getItem(keyFor(conversationId, requestId));
  if (raw === null) throw new MissingPreviewOwnerIntent();
  let value: unknown;
  try { value = JSON.parse(raw); } catch { throw new Error("agent_preview_owner_intent_invalid"); }
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("agent_preview_owner_intent_invalid");
  const row = value as Record<string, unknown>;
  if (row.schemaVersion !== 1 || row.conversationId !== conversationId.toLowerCase() || row.requestId !== requestId.toLowerCase()
    || !(row.fixtureId === null || typeof row.fixtureId === "string" && uuid.test(row.fixtureId))) throw new Error("agent_preview_owner_intent_invalid");
  return {fixtureId: row.fixtureId as string | null};
}

/** Resolve only the exact typed writing paths; missing recovery intent fails closed. */
function writingOwnerIntentIdentity(command: string, args: Record<string, unknown>): {conversationId: string; requestId: string} | undefined {
  const steer = ["agent_steer_writing_turn", "agent_steer_context_turn", "agent_steer_note_turn", "agent_steer_paper_turn"].includes(command);
  const handoffResolution = ["agent_finish_steering_handoff", "agent_release_steering_handoff", "agent_cancel_unregistered_steering_handoff"].includes(command);
  const handoff = command === "agent_prepare_steering_handoff" || handoffResolution;
  const queueCancellation = command === "agent_cancel_writing_queue_preview";
  const path = command === "agent_prepare_writing_submission" || command === "agent_start_writing_submission"
    || command === "agent_prepare_writing_compaction" || command === "agent_start_compaction" || command === "agent_settle_writing_queued_compaction" || queueCancellation
    || command === "agent_prepare_context_submission" || command === "agent_start_context_submission"
    || command === "agent_prepare_context_submission_proven" || command === "agent_start_context_submission_guarded"
    || command === "agent_prepare_paper_submission" || command === "agent_prepare_paper_submission_proven" || command === "agent_start_paper_submission"
    || command === "agent_prepare_note_submission" || command === "agent_start_note_submission" || command === "agent_start_note_queued_submission"
    || command === "agent_prepare_reading_note_submission" || command === "agent_start_reading_note_submission"
    || ["agent_edit_writing_input_queue", "agent_edit_context_input_queue", "agent_edit_note_input_queue", "agent_edit_paper_input_queue"].includes(command) || steer || (handoff && !handoffResolution) ? null
    : handoffResolution ? "intent"
    : command === "agent_prepare_writing_queue_preview" || command === "agent_prepare_writing_queued_compaction" ? "submission"
    : command === "agent_prepare_note_queue_preview" ? "noteSubmission"
    : command === "agent_start_writing_queued_submission" ? "start" : undefined;
  if (path === undefined) return undefined;
  let request = args.request;
  if (!request || typeof request !== "object" || Array.isArray(request)) throw new Error("agent_preview_owner_invalid");
  if (path === "noteSubmission") {
    const submission = (request as Record<string, unknown>).submission;
    if (!submission || typeof submission !== "object" || Array.isArray(submission)) throw new Error("agent_preview_owner_invalid");
    request = (submission as Record<string, unknown>).request;
  } else if (path !== null) request = (request as Record<string, unknown>)[path];
  if (!request || typeof request !== "object" || Array.isArray(request)) throw new Error("agent_preview_owner_invalid");
  const {conversationId} = request as Record<string, unknown>;
  const requestId = (request as Record<string, unknown>)[steer ? "inputId" : handoff ? "operationId" : queueCancellation ? "previewRequestId" : "requestId"];
  if (typeof conversationId !== "string" || typeof requestId !== "string") throw new Error("agent_preview_owner_invalid");
  return {conversationId, requestId};
}

export function resolveWritingOwnerIntent(storage: Storage, command: string, args: Record<string, unknown>):
  {conversationId: string; fixtureId: string | null} | undefined {
  const identity = writingOwnerIntentIdentity(command, args);
  return identity ? {conversationId:identity.conversationId, ...readPreviewOwnerIntent(storage, identity.conversationId, identity.requestId)} : undefined;
}

/** A lost local owner is recoverable only from an exact native durable proof.
 * A missing receipt means unknown, never an ordinary (null) owner. */
export async function resolveOrRecoverPreviewOwnerIntent(preview: NativeAgentPreview, storage: Storage, command: string, args: Record<string, unknown>):
  Promise<{conversationId: string; fixtureId: string | null} | undefined> {
  const identity = writingOwnerIntentIdentity(command, args);
  if (!identity) return undefined;
  try {
    const owner = readPreviewOwnerIntent(storage,identity.conversationId,identity.requestId);
    const persistence = storagePersistence(storage);
    if (!persistence) throw new Error("agent_preview_owner_storage_unavailable");
    const generation = preview.bootstrap.generation;
    if (persistence.error) await persistence.retry();
    await persistence.flush();
    if (preview.bootstrap.generation !== generation) throw new Error("agent_preview_stale_owner");
    return {conversationId:identity.conversationId, ...owner};
  }
  catch (cause) { if (!(cause instanceof Error) || cause.message !== "agent_preview_owner_intent_missing") throw cause; }
  if (!storagePersistence(storage)) throw new Error("agent_preview_owner_storage_unavailable");
  const generation = preview.bootstrap.generation;
  const response = await preview.invoke("agent_preview_read_owner_proof", {request:{...identity, expectedGeneration:generation}});
  if (!response || typeof response !== "object" || Array.isArray(response)) throw new Error("agent_preview_owner_proof_invalid");
  const row = response as Record<string, unknown>;
  if (row.schemaVersion !== 1 || row.generation !== generation || preview.bootstrap.generation !== generation
    || row.conversationId !== identity.conversationId.toLowerCase() || row.requestId !== identity.requestId.toLowerCase())
    throw new Error("agent_preview_owner_proof_invalid");
  if (row.status === "missing") throw new MissingPreviewOwnerIntent();
  if (row.status !== "found" || !(row.fixtureId === null || typeof row.fixtureId === "string" && uuid.test(row.fixtureId)))
    throw new Error("agent_preview_owner_proof_invalid");
  const fixtureId = typeof row.fixtureId === "string" ? row.fixtureId.toLowerCase() : null;
  const key = keyFor(identity.conversationId, identity.requestId), existing = storage.getItem(key);
  if (existing !== null) {
    if (readPreviewOwnerIntent(storage,identity.conversationId,identity.requestId).fixtureId !== fixtureId)
      throw new Error("agent_preview_owner_intent_conflict");
  } else storage.setItem(key, JSON.stringify({schemaVersion:1, conversationId:identity.conversationId.toLowerCase(), requestId:identity.requestId.toLowerCase(), fixtureId}));
  const persistence = storagePersistence(storage);
  if (persistence?.error) await persistence.retry();
  await flushStorage(storage);
  if (preview.bootstrap.generation !== generation) throw new Error("agent_preview_stale_owner");
  return {conversationId:identity.conversationId, fixtureId};
}

/** Freeze a new request before dispatch; recovery must read the existing record. */
export async function freezeNewPreviewOwnerIntent(
  preview: NativeAgentPreview | undefined, storage: Storage, conversationId: string, requestId: string,
): Promise<void> {
  await captureNewPreviewOwnerIntent(preview, storage, conversationId)?.(requestId);
}

/** Synchronously capture the owner at operation creation; the returned writer
 * can be retried and can freeze a rebased identity without changing owner. */
export function captureNewPreviewOwnerIntent(
  preview: NativeAgentPreview | undefined, storage: Storage, conversationId: string,
): ((requestId: string) => Promise<void>) | undefined {
  if (!preview?.bootstrap.supported.includes(NATIVE_PREVIEW_OWNER_CAPABILITY)) return undefined;
  if (!preview.snapshotConversationOwner) throw new Error("agent_preview_owner_unavailable");
  if (!storagePersistence(storage)) throw new Error("agent_preview_owner_storage_unavailable");
  const owner = preview.snapshotConversationOwner(conversationId);
  if (!uuid.test(conversationId)) throw new Error("agent_preview_owner_invalid");
  const attempted = new Set<string>();
  return async requestId => {
    if (!uuid.test(requestId)) throw new Error("agent_preview_owner_invalid");
    if (preview.bootstrap.generation !== owner.generation) throw new Error("agent_preview_stale_owner");
    const key = keyFor(conversationId, requestId), existing = storage.getItem(key);
    if (existing !== null) {
      if (!attempted.has(requestId.toLowerCase()) || readPreviewOwnerIntent(storage, conversationId, requestId).fixtureId !== owner.fixtureId)
        throw new Error("agent_preview_owner_intent_conflict");
    } else {
      attempted.add(requestId.toLowerCase());
      storage.setItem(key, JSON.stringify({schemaVersion: 1, conversationId: conversationId.toLowerCase(),
        requestId: requestId.toLowerCase(), fixtureId: owner.fixtureId}));
    }
    const persistence = storagePersistence(storage);
    if (persistence?.error) await persistence.retry();
    await flushStorage(storage);
    if (preview.bootstrap.generation !== owner.generation) throw new Error("agent_preview_stale_owner");
  };
}
