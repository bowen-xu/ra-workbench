import type { DocumentCheckpoint, DocumentCommit, DocumentSnapshot } from "@research-assistant/core/hbs";
import { decodeStructuredResource, structuredResourceDiffFields, type StructuredResource } from "@research-assistant/core/structured-resources";
import { assertMockStructuredContent, mockPendingFields } from "./mockStructuredReview";

export interface MockStructuredHbsPort {
  read(resourceId: string): Promise<unknown>;
  commit(resourceId: string, request: DocumentCommit<unknown>): Promise<unknown>;
  readPreservation(resourceId: string): Promise<unknown>;
  savePreservation(resourceId: string, checkpoint: unknown): Promise<unknown>;
}
export interface MockStructuredHbsState {
  resources: StructuredResource[];
  versions: Record<string, StructuredResource>;
  baselineReviewed: Record<string, readonly string[]>;
  hbsReceipts?: Record<string, {readonly fingerprint: string; readonly result: unknown}>;
  hbsPreservations?: Record<string, {readonly revision: number; readonly checkpoint: DocumentCheckpoint<unknown>}>;
}
function failure(code: string, message: string, requestId?: string): Error {
  return Object.assign(new Error(`${code}：${message}`), {code, ...(requestId ? {requestId} : {})});
}
function object(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value);
}
function canonical(value: unknown): string {
  return JSON.stringify(value, (_key, value: unknown) => object(value)
    ? Object.fromEntries(Object.entries(value).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)) : value);
}
const bytes = (value: unknown): number => new TextEncoder().encode(canonical(value)).byteLength;
function document(resource: StructuredResource, value: unknown, code: string): unknown {
  try {
    if (value !== null) assertMockStructuredContent(resource.kind, value);
    if (bytes(value) > 8 * 1024 * 1024) throw new Error("正文超过容量限制");
    return structuredClone(value);
  } catch (reason) { throw failure(code, reason instanceof Error ? reason.message : "资源正文无效"); }
}
function checkpoint(resource: StructuredResource, value: unknown): DocumentCheckpoint<unknown> {
  const code = "MODEL_RECOVERY_INVALID";
  if (bytes(value) > 64 * 1024 * 1024) throw failure("MODEL_REQUEST_TOO_LARGE", "恢复请求超过容量限制");
  if (!object(value) || value.format !== 1 || value.kind !== "structured" || !object(value.observed)
    || !Number.isSafeInteger(value.observed.version) || Number(value.observed.version) < 0 || !Array.isArray(value.ordinary)
    || Object.keys(value).some(key => !["format", "kind", "observed", "content", "ancestor", "reviewAncestor", "baselineAncestor", "ordinary"].includes(key))) {
    throw failure(code, "恢复格式、类型或修订无效");
  }
  if (value.ordinary.some(change => !object(change) || typeof change.id !== "string" || !Array.isArray(change.path) || !Array.isArray(change.beforePath))) {
    throw failure(code, "普通输入投影范围无效");
  }
  return {format: 1, kind: "structured", observed: {version: Number(value.observed.version),
    saved: document(resource, value.observed.saved, code), baseline: document(resource, value.observed.baseline, code)},
    content: document(resource, value.content, code), ancestor: document(resource, value.ancestor, code),
    baselineAncestor: document(resource, value.baselineAncestor, code),
    ...("reviewAncestor" in value ? {reviewAncestor: document(resource, value.reviewAncestor, code)} : {}),
    ordinary: structuredClone(value.ordinary) as DocumentCheckpoint<unknown>["ordinary"]};
}
const snapshot = (resource: StructuredResource): DocumentSnapshot<unknown> => ({version: resource.revision,
  saved: structuredClone(resource.content), baseline: structuredClone(resource.reviewBaseline)});

/** Mock only the persistence boundary; all diff, merging, decisions and editor
 * history remain in the same HBS session used by the production application. */
export function createMockStructuredHbs<S extends MockStructuredHbsState>(ports: {
  state(): S;
  persist(next: S): Promise<void>;
  flush(): Promise<void>;
  notify?(resource: StructuredResource): void;
}): MockStructuredHbsPort {
  function resource(id: string): StructuredResource {
    const value = ports.state().resources.find(resource => resource.id === id);
    if (!value) throw failure("MODEL_NOT_FOUND", "找不到结构化资源");
    return decodeStructuredResource(structuredClone(value));
  }
  return {
    read: async id => snapshot(resource(id)),
    commit: async (id, request) => {
      if (!object(request) || typeof request.id !== "string" || !request.id || request.id.length > 256
        || !Number.isSafeInteger(request.expectedVersion) || request.expectedVersion < 0
        || Object.keys(request).some(key => !["id", "expectedVersion", "saved", "baseline"].includes(key))) {
        throw failure("MODEL_INVALID_REQUEST", "请求身份或版本无效");
      }
      const fingerprint = canonical(request), key = JSON.stringify([id, request.id]), state = ports.state();
      const prior = state.hbsReceipts?.[key];
      if (prior) {
        if (prior.fingerprint !== fingerprint) throw failure("MODEL_REQUEST_ID_REUSED", "请求身份不能更改内容", request.id);
        await ports.flush(); ports.notify?.(resource(id)); return structuredClone(prior.result);
      }
      const current = resource(id);
      if (current.revision !== request.expectedVersion) return {type: "conflict", requestId: request.id,
        snapshot: snapshot(current), code: "MODEL_VERSION_CONFLICT"};
      if (current.draft) throw failure("MODEL_DRAFT_PENDING", "资源仍有修复草稿，请先在原草稿流程处理；正文、基线与草稿均已保留", request.id);
      let saved: unknown, baseline: unknown;
      try { saved = document(current, request.saved, "MODEL_INVALID_DOCUMENT"); baseline = document(current, request.baseline, "MODEL_INVALID_BASELINE"); }
      catch (reason) { throw Object.assign(reason instanceof Error ? reason : failure("MODEL_INVALID_DOCUMENT", "资源正文无效"), {requestId: request.id}); }
      const changed = mockPendingFields(saved, current.content), pending = mockPendingFields(saved, baseline);
      const oldBaseline = structuredResourceDiffFields(current.reviewBaseline), newBaseline = structuredResourceDiffFields(baseline);
      const at = (fields: typeof oldBaseline, id: string): unknown => fields.find(field => field.id === id)?.value ?? null;
      const protectedFields = new Set((state.baselineReviewed[id] ?? []).filter(id => canonical(at(oldBaseline, id)) === canonical(at(newBaseline, id))));
      for (const field of current.reviewedFieldIds) if (changed.includes(field)) protectedFields.add(field);
      const next: StructuredResource = {...current, content: saved, reviewBaseline: baseline, revision: current.revision + 1,
        pendingFieldIds: pending, reviewedFieldIds: current.reviewedFieldIds.filter(id => !changed.includes(id)),
        ...(current.kind === "readingTemplate" && object(saved) && typeof saved.name === "string" ? {title: saved.name} : {})};
      const result = {type: "receipt", requestId: request.id, snapshot: snapshot(next)};
      await ports.persist({...state, resources: [...state.resources.filter(resource => resource.id !== id), next],
        versions: {...state.versions, [`${id}:${next.revision}`]: structuredClone(next)},
        baselineReviewed: {...state.baselineReviewed, [id]: [...protectedFields].filter(id => pending.includes(id))},
        hbsReceipts: {...state.hbsReceipts, [key]: {fingerprint, result}}});
      ports.notify?.(next);
      return structuredClone(result);
    },
    readPreservation: async id => {
      const current = resource(id), saved = ports.state().hbsPreservations?.[id];
      return saved ? checkpoint(current, saved.checkpoint) : null;
    },
    savePreservation: async (id, raw) => {
      const current = resource(id), value = checkpoint(current, raw), state = ports.state();
      const revision = (state.hbsPreservations?.[id]?.revision ?? 0) + 1;
      await ports.persist({...state, hbsPreservations: {...state.hbsPreservations, [id]: {revision, checkpoint: value}}});
      return {revision};
    },
  };
}
