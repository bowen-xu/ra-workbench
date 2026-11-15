import type {NotePageId} from "@research-assistant/core/notes";
import {NotesGatewayError, type NoteSourceLabel} from "@research-assistant/data/notes";
import {readState} from "./browserNotesStorage";

/** The index needs label snapshots, not target existence checks or note bodies. */
export async function browserNoteSourceLabels(pageIds: readonly NotePageId[]): Promise<readonly NoteSourceLabel[]> {
  if (pageIds.length > 500) throw new NotesGatewayError("invalid_input", "note_source_labels_limit：一次最多读取 500 篇笔记的来源标签。");
  if (!pageIds.length) return [];
  const selected = new Set(pageIds), labels = new Map<NotePageId, string[]>();
  const state = await readState({metadataOnly: true});
  for (const link of state.links) {
    if (!selected.has(link.sourcePageId) || link.target.kind !== "paper" && link.target.kind !== "manuscript" || !link.target.labelSnapshot) continue;
    const values = labels.get(link.sourcePageId) ?? [];
    values.push(link.target.labelSnapshot); labels.set(link.sourcePageId, values);
  }
  return [...selected].flatMap(pageId => {
    const values = labels.get(pageId);
    return values ? [{pageId, label: values.join(" · ")}] : [];
  });
}
