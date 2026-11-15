import type { ComponentProps } from "react";
import type { SideMenuController } from "@blocknote/react";

type SideMenuOptions = NonNullable<ComponentProps<typeof SideMenuController>["floatingUIOptions"]>;
type FloatingOptions = NonNullable<SideMenuOptions["useFloatingOptions"]>;
type FloatingMiddleware = Exclude<NonNullable<FloatingOptions["middleware"]>[number], null | undefined | false>;

export function blockReviewSnapshot(block: HTMLElement): HTMLElement | undefined {
  const anchorId = block.dataset.blockReviewAnchor;
  if (!anchorId || block.dataset.id !== anchorId) return undefined;
  return Array.from(block.closest(".bn-editor")?.querySelectorAll<HTMLElement>("[data-block-review-anchor-for]") ?? [])
    .find(element => element.dataset.blockReviewAnchorFor === anchorId);
}

/** A review host can put a readonly snapshot before a live block. Only its first
 * candidate root is explicitly linked to that snapshot. Keep the upstream X
 * coordinate and all ordinary/nested block anchors, but align this root's side
 * controls to the start of the displayed old/new block. Both markers are UI
 * presentation attributes; they never enter the editor document or storage.
 */
export const blockSideMenuReviewAnchor: FloatingMiddleware = {
  name: "block-review-anchor",
  fn: ({ y, elements }) => {
    const reference = elements.reference;
    const context = "contextElement" in reference ? reference.contextElement : reference;
    if (!(context instanceof Element)) return {};
    const block = context.closest<HTMLElement>(".bn-block-outer[data-id]");
    const snapshot = block ? blockReviewSnapshot(block) : undefined;
    if (!snapshot) return {};
    return { y: y + snapshot.getBoundingClientRect().top - reference.getBoundingClientRect().top };
  },
};
