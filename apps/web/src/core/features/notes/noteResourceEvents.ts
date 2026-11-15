import type { NoteMetadataAction, NoteMetadataBatchReceipt } from "@research-assistant/core/notes";

/** Committed storage receipts consumed by document hosts and library views. */
export const NOTE_METADATA_CHANGED = "research-assistant:note-metadata-changed";
export const NOTE_PAGES_DELETED = "research-assistant:note-pages-deleted";
export type NoteMetadataChanged = Pick<NoteMetadataBatchReceipt, "pages" | "bodyChangedIds"> & { readonly action: NoteMetadataAction };
