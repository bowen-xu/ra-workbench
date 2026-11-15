import {isFixedBlock} from "./blockEditingConstraints";
/** Presentation snapshots can retain BlockNote classes inside a live parent.
 * Stop at the nearest block, rather than climbing past an ID-less snapshot to
 * the parent's identity. Read-only atom content can still belong to a real block.
 */
export function getBlockContentOwner(editor: HTMLElement, content: HTMLElement, options: {readonly includeFixed?: boolean} = {}): HTMLElement | undefined {
  if (!options.includeFixed && isFixedBlock({type: content.dataset.contentType ?? ""})) return;
  if (!editor.contains(content) || content.closest(".bn-editor") !== editor) return;
  const block = content.closest<HTMLElement>(".bn-block");
  return block?.dataset.id && editor.contains(block) && block.closest(".bn-editor") === editor ? block : undefined;
}
