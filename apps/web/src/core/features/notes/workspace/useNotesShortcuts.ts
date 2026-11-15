import { useEffect } from "react";

import type {NotesShortcutOptions} from "@research-assistant/workbench-sdk/note-presentation";

export function useNotesShortcuts(options: NotesShortcutOptions): void {
  useEffect(() => {
    const handleShortcut = (event: KeyboardEvent): void => {
      if (options.scopeRef && (!(event.target instanceof Node) || !options.scopeRef.current?.contains(event.target))) return;
      if (!(event.metaKey || event.ctrlKey)) return;
      const key = event.key.toLocaleLowerCase();
      if (key === "k") {
        event.preventDefault();
        options.onSearch?.();
        options.searchInputRef.current?.focus();
      } else if (key === "s") {
        event.preventDefault();
        options.onSave();
      } else if (key === "n" && !event.shiftKey) {
        event.preventDefault();
        options.onCreatePage();
      }
    };
    window.addEventListener("keydown", handleShortcut);
    return () => window.removeEventListener("keydown", handleShortcut);
  }, [options]);
}
