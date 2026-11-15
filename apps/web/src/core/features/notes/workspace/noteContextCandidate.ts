import type {AgentContextSelection} from "@research-assistant/core/agent";
import type {NoteLink, NotePage, ReadingNoteBlock} from "@research-assistant/core/notes";

/** Reading and whole-paper notes share their paper's managed context scope.
 * The recommendation gateway resolves the current label and validates availability. */
export function noteContextCandidate(page: NotePage, reading: ReadingNoteBlock | undefined, links: readonly NoteLink[]): AgentContextSelection {
  const source = reading?.pageId === page.id ? reading.source : undefined;
  const paperId = source?.paperId ?? page.literaturePaperId;
  if (paperId) {
    const label = source?.paperTitle ?? links.find(link => link.sourcePageId === page.id
      && link.sourceBlockId === undefined && link.relation === "derived_from"
      && link.target.kind === "paper" && link.target.id === paperId)?.target.labelSnapshot;
    return {resource: {kind: "paper", resourceId: paperId}, label: label || "文献"};
  }
  return {resource: {kind: "note", resourceId: page.id}, label: page.title || "未命名笔记"};
}
