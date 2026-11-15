interface SelectionPoints {
  readonly anchorNode: Node | null;
  readonly anchorOffset: number;
  readonly focusNode: Node | null;
  readonly focusOffset: number;
}
interface Gesture {
  readonly pointerId: number;
  readonly editor: HTMLElement;
  readonly x: number;
  readonly y: number;
  readonly selection: SelectionPoints | undefined;
  moved: boolean;
  released: boolean;
}

function selectionPoints(document: Document): SelectionPoints | undefined {
  const selection = document.getSelection();
  return selection ? {anchorNode: selection.anchorNode, anchorOffset: selection.anchorOffset,
    focusNode: selection.focusNode, focusOffset: selection.focusOffset} : undefined;
}

function selectionChanged(document: Document, before: SelectionPoints | undefined): boolean {
  const selection = document.getSelection();
  return !!selection && !selection.isCollapsed && (!before
    || before.anchorNode !== selection.anchorNode || before.anchorOffset !== selection.anchorOffset
    || before.focusNode !== selection.focusNode || before.focusOffset !== selection.focusOffset);
}

/** Observe native text selection without capturing pointers or changing ranges.
 * The application click owner consumes only the matching pointer gesture;
 * keyboard/programmatic activation and the next fresh click stay independent. */
export function trackLinkTextGesture(document: Document): {consumeClick: (event: MouseEvent) => boolean; dispose: () => void} {
  let gesture: Gesture | undefined;
  const clear = () => {gesture?.editor.removeAttribute("data-link-text-selecting"); gesture = undefined;};
  const down = (event: PointerEvent) => {
    clear();
    if (event.button !== 0 || !event.isPrimary) return;
    const anchor = event.composedPath().find((node): node is HTMLAnchorElement => node instanceof HTMLAnchorElement
      && node.matches('a[data-inline-content-type="link"]'));
    const editor = anchor?.closest<HTMLElement>(".bn-editor");
    if (!editor) return;
    gesture = {pointerId: event.pointerId, editor, x: event.clientX, y: event.clientY,
      selection: selectionPoints(document), moved: false, released: false};
  };
  const move = (event: PointerEvent) => {
    if (!gesture || gesture.released || gesture.pointerId !== event.pointerId) return;
    gesture.moved ||= Math.hypot(event.clientX - gesture.x, event.clientY - gesture.y) >= 6;
    if (gesture.moved) gesture.editor.setAttribute("data-link-text-selecting", "");
  };
  const up = (event: PointerEvent) => {
    move(event);
    if (gesture?.pointerId === event.pointerId) {
      gesture.released = true;
      gesture.editor.removeAttribute("data-link-text-selecting");
    }
  };
  const selection = () => {
    if (gesture && !gesture.released && selectionChanged(document, gesture.selection)) {
      gesture.editor.setAttribute("data-link-text-selecting", "");
    }
  };
  const cancel = (event: PointerEvent) => {if (gesture?.pointerId === event.pointerId) clear();};
  const visibility = () => {if (document.visibilityState === "hidden") clear();};
  document.addEventListener("pointerdown", down, true);
  document.addEventListener("pointermove", move, true);
  document.addEventListener("pointerup", up, true);
  document.addEventListener("pointercancel", cancel, true);
  document.addEventListener("selectionchange", selection);
  document.addEventListener("visibilitychange", visibility);
  document.defaultView?.addEventListener("blur", clear);
  return {
    consumeClick(event) {
      const current = gesture; clear();
      if (!current?.released || event.button !== 0 || event.detail === 0) return false;
      return current.moved || selectionChanged(document, current.selection);
    },
    dispose() {
      clear();
      document.removeEventListener("pointerdown", down, true);
      document.removeEventListener("pointermove", move, true);
      document.removeEventListener("pointerup", up, true);
      document.removeEventListener("pointercancel", cancel, true);
      document.removeEventListener("selectionchange", selection);
      document.removeEventListener("visibilitychange", visibility);
      document.defaultView?.removeEventListener("blur", clear);
    },
  };
}
