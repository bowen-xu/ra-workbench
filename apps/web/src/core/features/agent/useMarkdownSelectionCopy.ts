import {useEffect, type RefObject} from "react";
import {selectionClipboard} from "../../components/rich-text/selectionClipboard";
import {notify} from "../../services/notifications";

interface Registry {readonly roots: Set<HTMLElement>; readonly dispose: () => void;}
const registries = new WeakMap<Document, Registry>();
function createRegistry(document: Document): Registry {
  const roots = new Set<HTMLElement>();
  const copy = (event: ClipboardEvent) => {
    // Input/editor copy may leave a previous document Range visible to the API.
    // Its focused control owns the gesture, including execCommand fallbacks.
    if (event.target instanceof Element && event.target.closest('input, textarea, [contenteditable="true"], .cm-editor')) return;
    const selected = document.getSelection();
    if (event.defaultPrevented || !event.clipboardData || !selected || selected.isCollapsed || selected.rangeCount !== 1) return;
    const range = selected.getRangeAt(0);
    const ordered = [...roots].filter(root => root.isConnected && range.intersectsNode(root))
      .sort((a, b) => a.compareDocumentPosition(b) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1);
    const first = ordered[0], last = ordered.at(-1);
    const owns = (root: HTMLElement | undefined, node: Node) => root?.contains(node)
      || root?.closest(".writing-agent-message")?.contains(node);
    if (!owns(first, range.startContainer) || !owns(last, range.endContainer)) return;
    if (!first) return;
    // Mixed user/assistant copies belong to the existing bound-reference owner.
    const transcript = first.closest(".writing-agent-transcript");
    if (transcript && [...transcript.querySelectorAll(".agent-user-message-text")].some(node => range.intersectsNode(node))) return;
    try {
      const fragments = ordered.map(root => selectionClipboard(root, range));
      event.clipboardData.setData("text/plain", fragments.map(part => part.text).join("\n\n"));
      event.clipboardData.setData("text/html", fragments.map(part => part.html).join("\n"));
      event.preventDefault();
    } catch {
      // A failed semantic copy must not silently export KaTeX's duplicate text.
      event.preventDefault();
      notify.error("AGENT_SELECTION_COPY_FAILED：无法完整复制选区，请重试或缩小选择范围。");
    }
  };
  document.addEventListener("copy", copy);
  return {roots, dispose: () => document.removeEventListener("copy", copy)};
}
export function useMarkdownSelectionCopy(element: RefObject<HTMLElement | null>): void {
  useEffect(() => {
    const root = element.current;
    if (!root) return;
    const document = root.ownerDocument;
    let registry = registries.get(document);
    if (!registry) {registry = createRegistry(document); registries.set(document, registry);}
    registry.roots.add(root);
    return () => {
      registry.roots.delete(root);
      if (!registry.roots.size) {registry.dispose(); registries.delete(document);}
    };
  }, [element]);
}
