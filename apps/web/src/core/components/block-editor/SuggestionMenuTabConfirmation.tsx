import { SuggestionMenu } from "@blocknote/core/extensions";
import { useBlockNoteEditor, useEditorState, useExtension } from "@blocknote/react";
import { useEffect } from "react";
import { getEditorOverlayTarget } from "./editorSurface";

/** Shared suggestion policy; upstream still owns menu navigation and insertion. */
export function SuggestionMenuTabConfirmation({ triggerCharacter }: { readonly triggerCharacter: string }): null {
  const editor = useBlockNoteEditor();
  const menu = useExtension(SuggestionMenu);
  const root = useEditorState({ on: "mount", selector: context => context.editor?.domElement });
  useEffect(() => {
    const host = root?.closest(".block-editor");
    if (!root || !host) return;
    const keyDown = (event: Event) => {
      if (!(event instanceof KeyboardEvent) || !editor.isEditable || !menu.shown()
        || menu.store.state?.triggerCharacter !== triggerCharacter
        || !(event.target instanceof Element) || !root.contains(event.target)
        || event.target.closest('input, textarea, button, select, [contenteditable="false"]')
        || event.ctrlKey || event.metaKey || event.altKey) return;
      if (event.isComposing || event.keyCode === 229) {
        // Let native composition finish without reaching either the menu's
        // confirmation listener or the editor's ordinary Enter keymap.
        if (event.key === "Enter") event.stopImmediatePropagation();
        return;
      }
      if (event.key === "Tab" && !event.shiftKey) {
        event.preventDefault(); event.stopImmediatePropagation();
        const menuHost = getEditorOverlayTarget(root) ?? host;
        const selected = Array.from(menuHost.querySelectorAll<HTMLElement>('.bn-suggestion-menu [role="option"][aria-selected="true"]'))
          .find(option => option.getClientRects().length > 0 && getComputedStyle(option).visibility !== "hidden"
            && !option.closest('[hidden], [aria-hidden="true"]'));
        selected?.click();
      } else if (event.key === "Enter") {
        menu.closeMenu();
        event.stopImmediatePropagation();
        // The upstream menu uses a DOM capture listener for Enter. Bypass that
        // listener, then delegate the original key to ProseMirror's public
        // keymap pipeline (including paragraph/list/Shift+Enter behavior).
        const view = editor.prosemirrorView;
        if (view.someProp("handleKeyDown", handler => handler(view, event))) event.preventDefault();
      }
    };
    host.addEventListener("keydown", keyDown, true);
    return () => host.removeEventListener("keydown", keyDown, true);
  }, [editor, menu, root, triggerCharacter]);
  return null;
}
