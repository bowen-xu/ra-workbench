/** Portaled editor controls retain their editor's identity outside its DOM root. */
export function getEditorOverlayTarget(editor: HTMLElement): HTMLElement | undefined {
  const instance = editor.closest<HTMLElement>("[data-editor-instance]")?.dataset.editorInstance;
  if (!instance) return undefined;
  return [...editor.ownerDocument.querySelectorAll<HTMLElement>("[data-editor-overlay-owner]")]
    .find(target => target.dataset.editorOverlayOwner === instance);
}

export function ownsEditorElement(editor: HTMLElement, element: Element): boolean {
  return editor.closest(".block-editor")?.contains(element) === true
    || getEditorOverlayTarget(editor)?.contains(element) === true;
}

/** Retained editors keep their DOM and observers while hidden or inert. */
export function isEditorSurfaceActive(editor: HTMLElement): boolean {
  return editor.closest(".block-editor")?.closest("[hidden], [inert]") === null;
}

/** Layout wrappers around this surface provide its whitespace too. Unrelated
 * siblings (headers, relationships, controls) never inherit that ownership. */
export function ownsEditorPaneBackground(editor: HTMLElement, target: Element, pane: HTMLElement): boolean {
  const surface = editor.closest(".block-editor");
  return surface !== null && isEditorSurfaceActive(editor) && pane.contains(target)
    && (target === pane || target.contains(surface));
}

/** Only interactive surfaces in this scroll pane share its outer margins.
 * Hidden retained tabs and editors with their own scrolling do not take them. */
export function isSingleEditorPane(editor: HTMLElement, pane: HTMLElement): boolean {
  const surfaces = [...pane.querySelectorAll<HTMLElement>(".block-editor")];
  if (pane.matches(".block-editor")) surfaces.unshift(pane);
  const active = surfaces.filter(surface => surface.closest("[hidden], [inert]") === null
    && (getEditorScrollContainer(surface.querySelector<HTMLElement>(".bn-editor") ?? surface) ?? surface) === pane);
  return active.length === 1 && active[0] === editor.closest(".block-editor");
}

/** Global observers may see moves over unrelated surfaces. Ownership follows
 * the actual event target, not an upstream coordinate projection into a block.
 * Only the pane and this editor's ancestor layout backgrounds are whitespace. */
export function ownsEditorHoverEvent(editor: HTMLElement, event: Event): boolean {
  const target = event.composedPath().find((node): node is Element => node instanceof Element)
    ?? (event.target instanceof Element ? event.target : undefined);
  if (!target || !isEditorSurfaceActive(editor)) return false;
  const surface = editor.closest(".block-editor");
  const owner = target.closest(".block-editor");
  if (owner) return owner === surface;
  if (getEditorOverlayTarget(editor)?.contains(target)) return true;
  const pane = getEditorScrollContainer(editor);
  return pane !== null && ownsEditorPaneBackground(editor, target, pane) && isSingleEditorPane(editor, pane);
}

/** Discover the host's scrolling surface without depending on feature classes.
 * Hosts with nonstandard scrolling may mark their container explicitly.
 */
export function getEditorScrollContainer(editor: HTMLElement): HTMLElement | null {
  const marked = editor.closest<HTMLElement>("[data-block-editor-scroll-container]");
  if (marked !== null) return marked;
  const window = editor.ownerDocument.defaultView;
  for (let node = editor.parentElement; node !== null; node = node.parentElement) {
    if (window !== null && /auto|scroll|overlay/u.test(window.getComputedStyle(node).overflowY)) return node;
  }
  const page = editor.ownerDocument.scrollingElement;
  return page instanceof HTMLElement && page.scrollHeight > page.clientHeight ? page : null;
}
