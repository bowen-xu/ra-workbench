import { isDependentReadingNote, noteDocumentPlainText, type NotePage, type NotePageId, type ReadingNoteBlock } from "@research-assistant/core/notes";
import { NotesGatewayError, type NotesGateway, type ReadingNoteListQuery } from "@research-assistant/data/notes";
import { readState,readReadingNoteState, readingNotesFromState, type BrowserNotesState } from "./browserNotesStorage";

type ReadingMetadata = Omit<ReadingNoteBlock, "body" | "pageId"> & {readonly pageId: NotePageId};
function readingMetadata(state: BrowserNotesState): readonly ReadingMetadata[] {
  const pages = new Map(state.pages.map(page => [page.id, page]));
  return state.readingSources.flatMap(source => {
    const page = pages.get(source.pageId);
    return page ? [{...page, ...source}] : [];
  });
}
function eligibleReadingNotes(state: BrowserNotesState, query: ReadingNoteListQuery): readonly ReadingMetadata[] {
  return readingMetadata(state).filter(note => note.status === (query.status ?? "active")
    && (query.paperId === undefined || note.source.paperId === query.paperId)
    && (query.noteType === undefined || note.noteType === query.noteType)
    && (query.sourceState === undefined || note.source.state === query.sourceState)
    && (query.projectId === undefined || note.projectIds.includes(query.projectId))
    && (query.tagId === undefined || note.tagIds.includes(query.tagId))
    && (query.cursor === undefined || note.id > query.cursor));
}
function linkedPages(state: BrowserNotesState, paperId: string): readonly NotePage[] {
  const ids = new Set(state.links.filter(link => link.target.kind === "paper" && link.target.id === paperId
    && link.relation === "derived_from" && link.sourceBlockId === undefined).map(link => link.sourcePageId));
  return state.pages.filter(page => page.status === "active" && ids.has(page.id));
}

/** Read projections select their bodies in the metadata transaction. They never
 * load revision history, conflict receipts or unrelated ordinary documents. */
export function browserNoteReads():
  Pick<NotesGateway, "listReadingNotes" | "getReadingNote" | "listLiteratureNotes"> {
  return {
    async listReadingNotes(query = {}) {
      const state = await readState({pageIds: metadata => eligibleReadingNotes(metadata, query).map(note => note.pageId)});
      const eligible = new Set(eligibleReadingNotes(state, query).map(note => note.id));
      const needle = query.search?.trim().toLocaleLowerCase() ?? "";
      // Search and missing-body filtering precede the limit, preserving paging
      // even when an earlier metadata record has no readable document.
      return readingNotesFromState(state).filter(note => eligible.has(note.id)
        && (needle === "" || [note.body, note.source.quoteExact, note.source.paperTitle].join(" ").toLocaleLowerCase().includes(needle)))
        .sort((a, b) => a.id < b.id ? -1 : a.id > b.id ? 1 : 0).slice(0, query.limit ?? 500);
    },
    async getReadingNote(id) {
      const state = await readReadingNoteState(id);
      const note = readingNotesFromState(state).find(candidate => candidate.id === id || candidate.pageId === String(id));
      if (!note) throw new NotesGatewayError("not_found", "找不到阅读笔记");
      return note;
    },
    async listLiteratureNotes(paperId) {
      const state = await readState({pageIds: metadata => linkedPages(metadata, paperId).map(page => page.id)});
      const dependent = new Set(readingNotesFromState(state).filter(isDependentReadingNote).map(note => note.pageId));
      return linkedPages(state, paperId).filter(page => !dependent.has(page.id))
        .sort((a, b) => b.createdAt.localeCompare(a.createdAt) || b.position - a.position)
        .map(page => ({page, preview: state.documents[page.id] === undefined
          ? "" : noteDocumentPlainText(state.documents[page.id]!).slice(0, 300)}));
    },
  };
}
