import {expectTypeOf,it} from "vitest";
import type {NotesGateway} from "@research-assistant/data/notes";
import type {NoteManagementStorage} from "./noteStorage";
import type {getNoteManagementGateway} from "../public/noteStorage";
import type {useNoteDocumentSession,useNoteAcceptance} from "../public/noteDocuments";

it("keeps raw platform operations and single-document writes outside the library metadata port",()=>{
  expectTypeOf<NotesGateway>().toExtend<NoteManagementStorage>();
  expectTypeOf<ReturnType<typeof getNoteManagementGateway>>().toEqualTypeOf<NoteManagementStorage>();
  expectTypeOf<Extract<keyof NoteManagementStorage,"createPage"|"saveDocument"|"invoke"|"executeSql">>().toEqualTypeOf<never>();
});
it("lets document hosts use Core sessions without supplying an internal NotesClient or gateway",()=>{
  expectTypeOf<Extract<keyof Parameters<typeof useNoteDocumentSession>[0],"client">>().toEqualTypeOf<never>();
  expectTypeOf<Extract<keyof Parameters<typeof useNoteAcceptance>[0],"notes">>().toEqualTypeOf<never>();
});
