import {noteDocumentPlainText, notePageId, type NoteIndexItem} from "@research-assistant/core/notes";
import {projectNoteCatalogItemHeader, type NoteCatalogRuntime} from "@research-assistant/data/notes";
import {decodeWritingNoteValue, writingNoteTitle, type Manuscript, type ManuscriptFile} from "@research-assistant/writing";

export type WritingCatalogContent = {readonly noteId: string; readonly text: string} | {readonly error: string};
/** Parse committed bytes only. This projection is never an editable document. */
export function projectWritingCatalogContent(content: string): WritingCatalogContent {
  try {
    const value: unknown = JSON.parse(content);
    const row = value && typeof value === "object" ? value as Record<string,unknown> : undefined;
    if (row?.schemaVersion !== 1) return {error: "writing_note_version_invalid：不支持的写作笔记版本；原文件已保留。"};
    if (typeof row.noteId !== "string" || !row.noteId || row.noteId.length > 128) return {error: "writing_note_identity_invalid：写作笔记标识为空或过长；原文件已保留。"};
    const note = decodeWritingNoteValue(value);
    return {noteId: note.noteId, text: noteDocumentPlainText({schemaVersion: 1, pageId: notePageId("projection"), blocks: note.blocks})};
  } catch (reason) {
    const location = reason instanceof SyntaxError ? reason.message.match(/position\s+\d+|line\s+\d+\s+column\s+\d+/gu)?.join("; ") : undefined;
    // Core validators can mention source values. Keep safe categories and JSON
    // positions rather than echoing a malformed private block in diagnostics.
    return {error: `${reason instanceof SyntaxError ? "writing_note_json_invalid" : "writing_note_blocks_invalid"}：写作笔记无法生成查询投影；原文件已保留（${reason instanceof Error ? reason.name : "invalid_record"}${location ? `; ${location}` : ""}）。`};
  }
}
export function writingCatalogId(manuscriptId: string, noteId: string): string { return `writing:${JSON.stringify([manuscriptId,noteId])}`; }
export function projectWritingCatalogHeader(file: ManuscriptFile, manuscript: Manuscript, id: string, ordinal: number, runtime: NoteCatalogRuntime) {
  const title = writingNoteTitle(file.relativePath);
  const item: NoteIndexItem = {id, title, snippet: "", category: "dependent", owner: {kind: "manuscript", id: manuscript.id, label: manuscript.title},
    updatedAt: file.modifiedAt, status: manuscript.status, isFavorite: false, target: {kind: "writing", manuscriptId: manuscript.id, path: file.relativePath}};
  return projectNoteCatalogItemHeader({item, metadataText: `${title} ${manuscript.title}`, ordinal},runtime);
}
