export const APP_SELECTABLE_TEXT_SELECTOR = [
  "input",
  "textarea",
  "[contenteditable]:not([contenteditable='false'])",
  ".textLayer",
  ".cm-content",
  ".bn-editor",
  "[data-selectable-text]",
  "blockquote",
  "q",
  "pre",
  "code",
].join(",");

export function allowsAppTextSelection(target: EventTarget | null): boolean {
  const element = target instanceof Element
    ? target
    : target instanceof Node
      ? target.parentElement
      : null;
  return element !== null && element.closest(APP_SELECTABLE_TEXT_SELECTOR) !== null;
}
