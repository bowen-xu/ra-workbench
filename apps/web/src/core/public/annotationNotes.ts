import type { NotePageId } from "@research-assistant/core/notes";
import { readState } from "../features/notes/gateways/browserNotesStorage";
import { ensureBrowserAnnotationPage } from "../features/notes/gateways/browserNotesGateway";

/** Legacy browser-domain adapter port. It returns identities only, preserving
 * the original metadata-only read and same Core store; no internal state leaks. */
export async function listBrowserNotePageIds(): Promise<readonly NotePageId[]> {
  return (await readState({metadataOnly:true})).pages.map(page=>page.id);
}
export function ensureBrowserAnnotationNote(target: readonly [string,string], title: string) {
  return ensureBrowserAnnotationPage(target,title);
}
