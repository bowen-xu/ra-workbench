import { useEffect } from "react";
import { matchesShortcut, type ShortcutBindings } from "../settings/keyboardShortcuts";

/** Capture before editor keymaps; the application shell owns these two commands. */
export function useAgentCanvasShortcuts(bindings: ShortcutBindings, toggleChat: () => void, toggleConversations: () => void): void {
  useEffect(() => {
    const keydown = (event: KeyboardEvent): void => {
      if (event.defaultPrevented || event.repeat || event.isComposing || event.keyCode === 229
        || document.querySelector('dialog[open], [aria-modal="true"]')
        || (event.target instanceof Element && event.target.closest('[data-shortcut-recorder], [role="menu"], [role="listbox"], select'))) return;
      const action = matchesShortcut(event, bindings.toggleChat) ? toggleChat : matchesShortcut(event, bindings.toggleConversations) ? toggleConversations : undefined;
      if (!action) return;
      event.preventDefault(); event.stopPropagation(); action();
    };
    window.addEventListener("keydown", keydown, true);
    return () => window.removeEventListener("keydown", keydown, true);
  }, [bindings, toggleChat, toggleConversations]);
}
