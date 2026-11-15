import { decodeMockState } from "../../../core/runtime/mockStateCodec";

const FORMAT = "structured-resource-documents-v1";
const KINDS = new Set(["portrait", "structuredReading", "readingTemplate"]);
function isResource(value: unknown): value is Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const row = value as Record<string, unknown>;
  return typeof row.id === "string" && typeof row.kind === "string" && KINDS.has(row.kind)
    && typeof row.sourceUri === "string" && Number.isSafeInteger(row.revision)
    && "content" in row && "reviewBaseline" in row;
}

/** Intern immutable D/B documents before the existing preview compression pass. */
export function encodeStructuredState(value: unknown): string {
  const documents: string[] = [], indices = new Map<string, number>();
  function reference(document: unknown): number {
    const raw = JSON.stringify(document);
    if (raw === undefined) throw new Error("资源正文无法保存");
    const existing = indices.get(raw);
    if (existing !== undefined) return existing;
    const index = documents.length;
    indices.set(raw, index); documents.push(raw);
    return index;
  }
  const state = JSON.stringify(value, (_key, row: unknown) => isResource(row)
    ? {...row, content: reference(row.content), reviewBaseline: reference(row.reviewBaseline)} : row);
  return JSON.stringify({format: FORMAT, documents, state});
}

/** Existing plain mock snapshots remain readable; document payloads are opaque JSON. */
export function decodeStructuredState<T>(raw: string): T {
  const envelope = JSON.parse(raw) as {format?: unknown; documents?: unknown; state?: unknown};
  if (envelope?.format !== FORMAT) return decodeMockState<T>(raw);
  if (!Array.isArray(envelope.documents) || envelope.documents.some(value => typeof value !== "string")
    || typeof envelope.state !== "string") throw new Error("预览资源快照损坏，原记录已保留");
  const documents: unknown[] = envelope.documents.map(value => JSON.parse(value as string));
  function document(index: unknown): unknown {
    if (typeof index !== "number" || !Number.isSafeInteger(index) || index < 0 || index >= documents.length)
      throw new Error("预览资源正文引用损坏，原记录已保留");
    return structuredClone(documents[index]);
  }
  return JSON.parse(envelope.state, (_key, row: unknown) => isResource(row)
    ? {...row, content: document(row.content), reviewBaseline: document(row.reviewBaseline)} : row) as T;
}
