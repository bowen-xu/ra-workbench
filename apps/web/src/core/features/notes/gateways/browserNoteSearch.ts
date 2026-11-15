import type {NoteSearchPage, NoteSearchQuery} from "@research-assistant/data/notes";
import {queryBrowserNoteSearch} from "./browserNotesStorage";
import {validateNoteObjectSearch} from "./browserNoteObjectSearch";

export async function searchBrowserNotes(query: NoteSearchQuery): Promise<NoteSearchPage> {
  validateNoteObjectSearch(query);
  if (query.objectKinds?.length === 0) return {items: [],indexState: "ready"};
  return queryBrowserNoteSearch(query);
}
