import { getBlockContentOwner } from "./blockContentOwner";
import { isAttachedTextBlock, isFixedBlock } from "./blockEditingConstraints";
import { ownsEditorElement } from "./editorSurface";

/** DOM/identity only. Hosts retain AST, nesting permission and move commands. */
export interface BlockDragDom {
  readonly outerSelector: string;
  readonly contentSelector: string;
  readonly owns: (root: HTMLElement, hit: Element) => boolean;
  readonly contentId: (root: HTMLElement, content: HTMLElement, includeFixed?: boolean) => string | undefined;
  readonly idOf: (outer: HTMLElement) => string | undefined;
  readonly fixed: (content: HTMLElement) => boolean;
  readonly attached: (content: HTMLElement) => boolean;
  /** Hosts without child destinations can map the middle band to a sibling boundary. */
  readonly middlePlacement?: (content: HTMLElement) => "before" | "after" | "inside";
  readonly scope: (root: HTMLElement) => {readonly instance: string; readonly selector: string} | undefined;
  readonly rowSelector: (id: string) => string;
}

export const blockNoteDragDom: BlockDragDom = {
  outerSelector: ".bn-block-outer", contentSelector: ".bn-block-content",
  owns: ownsEditorElement,
  contentId: (root, content, includeFixed = false) => getBlockContentOwner(root, content, {includeFixed})?.dataset.id,
  idOf: outer => outer.dataset.id ?? outer.querySelector<HTMLElement>(".bn-block[data-id]")?.dataset.id,
  fixed: content => isFixedBlock({type: content.dataset.contentType ?? ""}),
  attached: content => isAttachedTextBlock({type: content.dataset.contentType ?? ""}),
  scope: root => {
    const instance = root.closest<HTMLElement>(".block-editor")?.dataset.editorInstance;
    return instance === undefined ? undefined : {instance, selector: `.block-editor[data-editor-instance="${CSS.escape(instance)}"]`};
  },
  rowSelector: id => `.bn-block-outer[data-id="${CSS.escape(id)}"]`,
};
