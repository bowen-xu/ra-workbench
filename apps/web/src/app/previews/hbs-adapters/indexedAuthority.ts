import type { DocumentCommit, DocumentSnapshot, HbsDocumentAdapter } from "@research-assistant/core/hbs";
import { decodeDocumentCheckpoint, decodeDocumentSnapshot } from "@research-assistant/data/hbs";
import type { HbsStoredTable } from "../../../core/components/hbs/HbsStorageRows";
import { openPreviewDatabase, PreviewStorageError, type PreviewDatabaseSnapshot } from "../../../core/runtime/previewIndexedDatabase";

/** Separate from both the App preview database and the browser's real notes. */
export const HBS_ADAPTER_DATABASE_PREFIX = "research-assistant.hbs-adapters.v1.";
type CommitReply<D> = { readonly type: "receipt" | "conflict"; readonly requestId: string; readonly snapshot: DocumentSnapshot<D> };
type StoredReceipt<D> = { readonly request: DocumentCommit<D>; readonly reply: CommitReply<D> };
function fail(code: string, message: string): never { throw Object.assign(new Error(`${code}：${message}`), {code}); }
function parse(raw: string | undefined): unknown {
  if (raw === undefined) fail("MODEL_STORAGE_CORRUPT", "预览存储缺少记录，原数据已保留。");
  try { return JSON.parse(raw); }
  catch { return fail("MODEL_STORAGE_CORRUPT", "预览存储记录无法解码，原数据已保留。"); }
}
function storageConflict(reason: unknown): boolean {
  return reason instanceof PreviewStorageError && reason.code === "preview_storage_conflict";
}
/** Compare the complete persisted checkpoint, including provenance and ordinary
 * ranges. JSON object property insertion order does not change its meaning. */
function checkpointFingerprint(value: unknown): string {
  return JSON.stringify(value, (_key, item: unknown) => item && typeof item === "object" && !Array.isArray(item)
    ? Object.fromEntries(Object.keys(item).sort().map(key => [key, (item as Record<string, unknown>)[key]])) : item);
}

/** IndexedDB owns every read and the atomic S/B/version/receipt transaction.
 * There is no in-memory authority and no fallback when a transaction fails. */
export async function createIndexedHbsAuthority<D>(adapter: HbsDocumentAdapter<D>, fixture: D) {
  if (!["text", "note", "structured"].includes(adapter.kind)) fail("MODEL_INVALID_KIND", "未知预览文档类型。");
  const decoded = adapter.decode(structuredClone(fixture));
  const id = adapter.kind === "note" ? String((decoded as {pageId?: unknown}).pageId)
    : adapter.kind === "text" ? "text" : "hbs-structured-example";
  const name = `${HBS_ADAPTER_DATABASE_PREFIX}${adapter.kind}`;
  const database = await openPreviewDatabase(name);
  const prefix = `document:${id}:`;
  const keys = {saved: `${prefix}S`, baseline: `${prefix}B`, version: `${prefix}version`, preservation: `${prefix}S′`};
  function document(value: unknown): D {
    const result = adapter.decode(value);
    if (adapter.kind === "note" && (result as {pageId?: unknown}).pageId !== id)
      fail("MODEL_INVALID_DOCUMENT", "不能跨页面写入笔记。");
    return result;
  }
  function snapshot(entries: ReadonlyMap<string, string>): DocumentSnapshot<D> {
    const value = decodeDocumentSnapshot({version: parse(entries.get(keys.version)), saved: parse(entries.get(keys.saved)), baseline: parse(entries.get(keys.baseline))}, adapter);
    document(value.saved); document(value.baseline); return value;
  }
  function request(input: unknown): DocumentCommit<D> {
    if (!input || typeof input !== "object") fail("MODEL_INVALID_REQUEST", "缺少保存请求。");
    const value = input as DocumentCommit<unknown>;
    if (typeof value.id !== "string" || !value.id.trim() || value.id.length > 256
      || !Number.isSafeInteger(value.expectedVersion) || value.expectedVersion < 0)
      fail("MODEL_INVALID_REQUEST", "请求身份或版本无效。");
    return {id: value.id, expectedVersion: value.expectedVersion, saved: document(value.saved), baseline: document(value.baseline)};
  }
  function receipt(input: unknown): StoredReceipt<D> {
    if (!input || typeof input !== "object") fail("MODEL_STORAGE_CORRUPT", "保存回执损坏。");
    const value = input as StoredReceipt<unknown>, original = request(value.request);
    if (!value.reply || value.reply.requestId !== original.id || !["receipt", "conflict"].includes(value.reply.type))
      fail("MODEL_STORAGE_CORRUPT", "保存回执身份无效。");
    return {request: original, reply: {...value.reply, snapshot: decodeDocumentSnapshot(value.reply.snapshot, adapter)}};
  }
  function checkpoint(input: unknown) {
    const value = decodeDocumentCheckpoint(input, adapter);
    for (const part of [value.content, value.ancestor, value.baselineAncestor, value.observed.saved, value.observed.baseline,
      ...(value.reviewAncestor === undefined ? [] : [value.reviewAncestor])]) document(part);
    return value;
  }
  function preservation(raw: string | undefined) {
    if (raw === undefined) return;
    const value = parse(raw) as {revision?: unknown; checkpoint?: unknown};
    if (!value || !Number.isSafeInteger(value.revision) || Number(value.revision) < 1)
      fail("MODEL_STORAGE_CORRUPT", "编辑保全修订号无效。");
    return {revision: Number(value.revision), checkpoint: checkpoint(value.checkpoint)};
  }
  async function bootstrap() {
    for (let attempt = 0; attempt < 8; attempt++) {
      const stored = await database.load();
      if (stored.entries.size) return {resourceId: id, snapshot: snapshot(stored.entries)};
      const initial = {version: 1, saved: decoded, baseline: decoded};
      try {
        await database.commit(stored.revision, new Map([
          [keys.saved, JSON.stringify(initial.saved)], [keys.baseline, JSON.stringify(initial.baseline)], [keys.version, "1"],
        ]), false);
        return {resourceId: id, snapshot: initial};
      } catch (reason) { if (!storageConflict(reason)) throw reason; }
    }
    fail("MODEL_STORAGE_BUSY", "预览存储持续变化，请重试；没有覆盖其他标签页。");
  }
  async function commit(input: unknown): Promise<CommitReply<D>> {
    const value = request(structuredClone(input)), receiptKey = `${prefix}receipt:${value.id}`;
    for (let attempt = 0; attempt < 8; attempt++) {
      const stored = await database.load();
      const previous = stored.entries.get(receiptKey);
      if (previous !== undefined) {
        const prior = receipt(parse(previous));
        if (prior.request.expectedVersion !== value.expectedVersion || !adapter.equal(prior.request.saved, value.saved) || !adapter.equal(prior.request.baseline, value.baseline))
          fail("MODEL_REQUEST_ID_REUSED", `请求 ${value.id} 已用于不同内容。`);
        return prior.reply;
      }
      const current = snapshot(stored.entries);
      if (current.version !== value.expectedVersion) return {type: "conflict", requestId: value.id, snapshot: current};
      if (!Number.isSafeInteger(current.version + 1)) fail("MODEL_INVALID_REQUEST", "文档修订号超出安全范围。");
      const next = {version: current.version + 1, saved: value.saved, baseline: value.baseline};
      const reply = {type: "receipt" as const, requestId: value.id, snapshot: next};
      try {
        await database.commit(stored.revision, new Map([
          [keys.saved, JSON.stringify(next.saved)], [keys.baseline, JSON.stringify(next.baseline)], [keys.version, String(next.version)],
          [receiptKey, JSON.stringify({request: value, reply})],
        ]), false);
        return reply;
      } catch (reason) { if (!storageConflict(reason)) throw reason; }
      // Re-read after storage contention. A changed HBS version becomes a
      // semantic conflict; an independent S′ write does not invalidate S/B.
    }
    return fail("MODEL_STORAGE_BUSY", `请求 ${value.id} 尚未提交，请重试原请求。`);
  }
  async function writePreservation(input?: unknown) {
    const value = input === undefined ? undefined : checkpoint(structuredClone(input));
    let original: string | undefined;
    for (let attempt = 0; attempt < 8; attempt++) {
      const stored = await database.load(); snapshot(stored.entries);
      const raw = stored.entries.get(keys.preservation);
      // The load has observed a durable transaction. Identical preservation
      // requests can reuse its receipt, including after an initial-open race.
      // Reset intentionally skips decoding so corrupt checkpoints are removable.
      const previous = value === undefined ? undefined : preservation(raw);
      if (value !== undefined && previous && checkpointFingerprint(previous.checkpoint) === checkpointFingerprint(value))
        return {revision: previous.revision};
      if (attempt === 0) original = raw;
      else if (raw !== original) fail("MODEL_PRESERVATION_CONFLICT", "其他标签页已修改编辑保全，未覆盖其内容。请重新核对。");
      const revision = (previous?.revision ?? 0) + 1;
      try {
        await database.commit(stored.revision, new Map([[keys.preservation, value === undefined ? null : JSON.stringify({revision, checkpoint: value})]]), false);
        return {revision};
      } catch (reason) { if (!storageConflict(reason)) throw reason; }
    }
    return fail("MODEL_STORAGE_BUSY", "编辑保全尚未提交，请重试。");
  }
  function tables(stored: PreviewDatabaseSnapshot, selected: readonly string[]): readonly HbsStoredTable[] {
    return [{table: `IndexedDB ${name} / entries`, columns: ["key", "value"], rows: selected.flatMap(key => {
      const value = stored.entries.get(key); return value === undefined ? [] : [{key, value}];
    })}];
  }
  async function call(operation: string, input?: unknown): Promise<unknown> {
    if (operation === "bootstrap") return bootstrap();
    if (operation === "commit") return commit(input);
    if (operation === "savePreservation") return writePreservation(checkpoint(input));
    if (operation === "clearPreservation") return writePreservation();
    const stored = await database.load(), current = snapshot(stored.entries);
    if (operation === "read") return current;
    if (operation === "readPreservation") return preservation(stored.entries.get(keys.preservation))?.checkpoint ?? null;
    if (operation === "inspectStorage") return {snapshot: current, storage: {
      S: tables(stored, [keys.version, keys.saved]), B: tables(stored, [keys.baseline]), "S′": tables(stored, [keys.preservation]),
    }};
    return fail("MODEL_INVALID_REQUEST", `未知预览操作：${operation}`);
  }
  try { await bootstrap(); }
  catch (reason) { database.close(); throw reason; }
  return {id, call, close: () => database.close()};
}
