import {decodeNotePageCatalogPage, type NotePageCatalogGateway} from "@research-assistant/data/notes";
import {queryBrowserPageCatalog} from "./browserNotesStorage";

/** Same storage owner as the ordinary note gateway, including preview isolation. */
export const browserNotePageCatalogGateway: NotePageCatalogGateway = {
  queryNotePageCatalog: async (query,options) => decodeNotePageCatalogPage(await queryBrowserPageCatalog({...query,search: query.search ?? ""},options),query.limit ?? 20),
};
