import { classifyManuscriptFile, parseManuscriptPath, type Manuscript, type ManuscriptFile, type ManuscriptFileContent, type RecoveryDraft, type WritingSession } from "@research-assistant/writing";

export interface StoredWritingProject { readonly manuscript: Manuscript; readonly directories: readonly string[] }
function corrupt(): never { throw Object.assign(new Error("writing_storage_corrupt：写作记录格式无效，原数据已保留。"), {code: "writing_storage_corrupt"}); }
export function record(value: unknown): Record<string, unknown> { return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : corrupt(); }
export function text(value: unknown): string { return typeof value === "string" ? value : corrupt(); }
function timestamp(value: unknown): string { const result = text(value); return Number.isFinite(Date.parse(result)) ? result : corrupt(); }
function hash(value: unknown): string { const result = text(value); return /^[a-f0-9]{64}$/u.test(result) ? result : corrupt(); }
function paths(value: unknown): readonly ReturnType<typeof parseManuscriptPath>[] { return Array.isArray(value) ? value.map(path => parseManuscriptPath(text(path))) : corrupt(); }
export function decodeWritingProject(value: unknown): StoredWritingProject {
  const row = record(value), source = record(row.manuscript);
  if (source.status !== "active" && source.status !== "archived") corrupt();
  return {manuscript: {id: text(source.id), title: text(source.title), rootPath: parseManuscriptPath(text(source.rootPath)),
    status: source.status, createdAt: timestamp(source.createdAt), updatedAt: timestamp(source.updatedAt),
    ...(source.projectId === undefined ? {} : {projectId: text(source.projectId)})}, directories: paths(row.directories)};
}
export function decodeWritingFile(value: unknown): ManuscriptFile {
  const row = record(value), path = parseManuscriptPath(text(row.relativePath));
  if (!Number.isSafeInteger(row.sizeBytes) || Number(row.sizeBytes) < 0 || row.kind !== classifyManuscriptFile(path)) corrupt();
  return {manuscriptId: text(row.manuscriptId), relativePath: path, kind: classifyManuscriptFile(path), sizeBytes: Number(row.sizeBytes), sha256: hash(row.sha256), modifiedAt: timestamp(row.modifiedAt)};
}
export function decodeWritingBody(value: unknown): ManuscriptFileContent {
  const row = record(value); return {relativePath: parseManuscriptPath(text(row.relativePath)), content: text(row.content), sha256: hash(row.sha256)};
}
export function decodeWritingSession(value: unknown): WritingSession {
  const row = record(value);
  return {manuscriptId: text(row.manuscriptId), openPaths: paths(row.openPaths),
    ...(row.activePath === undefined ? {} : {activePath: parseManuscriptPath(text(row.activePath))}),
    cursorState: record(row.cursorState), layoutState: record(row.layoutState), updatedAt: timestamp(row.updatedAt)};
}
export function decodeWritingDraft(value: unknown): RecoveryDraft {
  const row = record(value);
  return {manuscriptId: text(row.manuscriptId), relativePath: parseManuscriptPath(text(row.relativePath)),
    content: text(row.content), baseSha256: text(row.baseSha256), updatedAt: timestamp(row.updatedAt)};
}
