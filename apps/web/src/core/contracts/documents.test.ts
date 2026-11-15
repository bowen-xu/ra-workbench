import { expectTypeOf, it } from "vitest";
import type { NotesGateway } from "@research-assistant/data/notes";
import type { getNoteDocumentGateway } from "../public/documents";
import type { NoteDocumentAccess } from "./documents";

it("keeps library management and raw platform operations out of the public document port", () => {
  expectTypeOf<ReturnType<typeof getNoteDocumentGateway>>().toEqualTypeOf<NoteDocumentAccess>();
  expectTypeOf<NotesGateway>().toExtend<NoteDocumentAccess>();
  expectTypeOf<Extract<keyof NoteDocumentAccess,
    "createPage" | "listPages" | "movePage" | "reorderFavorites" | "invoke" | "connection"
  >>().toEqualTypeOf<never>();
});
