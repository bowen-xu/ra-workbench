import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { getAppStorage } from "../../runtime/appStorage";
import { notify } from "../../services/notifications";
import { defaultShortcuts, readShortcuts, sameShortcut, shortcutProblem, type ShortcutAction, type ShortcutBinding, type ShortcutBindings } from "./keyboardShortcuts";

export const SHORTCUT_STORAGE_KEY = "research-assistant.keyboard-shortcuts.v1";
interface Preferences {
  readonly bindings: ShortcutBindings;
  readonly update: (action: ShortcutAction, binding: ShortcutBinding | null) => string | undefined;
  readonly reset: () => void;
}
const Context = createContext<Preferences | null>(null);
function load(): ShortcutBindings {
  try { return readShortcuts(JSON.parse(getAppStorage().getItem(SHORTCUT_STORAGE_KEY) ?? "null")); }
  catch { return defaultShortcuts(); }
}
export function ShortcutPreferencesProvider({ children }: { readonly children: ReactNode }): React.JSX.Element {
  const [bindings, setBindings] = useState(load);
  useEffect(() => {
    const sync = (event: StorageEvent): void => {
      if (event.key === null || event.key.endsWith(SHORTCUT_STORAGE_KEY)) setBindings(load());
    };
    window.addEventListener("storage", sync);
    return () => window.removeEventListener("storage", sync);
  }, []);
  function save(next: ShortcutBindings): void {
    setBindings(next);
    try { getAppStorage().setItem(SHORTCUT_STORAGE_KEY, JSON.stringify(next)); }
    catch { notify.error("快捷键已生效，但无法保存设置；下次打开可能恢复默认值。"); }
  }
  function update(action: ShortcutAction, binding: ShortcutBinding | null): string | undefined {
    const problem = binding === null ? undefined : shortcutProblem(binding);
    if (problem) return problem;
    const other = action === "toggleChat" ? "toggleConversations" : "toggleChat";
    if (binding !== null && sameShortcut(binding, bindings[other])) return "这个快捷键已用于另一个操作，请换一个组合。";
    save({ ...bindings, [action]: binding });
    return undefined;
  }
  return <Context.Provider value={{ bindings, update, reset: () => save(defaultShortcuts()) }}>{children}</Context.Provider>;
}
export function useShortcutPreferences(): Preferences {
  const value = useContext(Context);
  if (!value) throw new Error("Shortcut preferences require an application provider.");
  return value;
}

/** Standalone chat panels use defaults; application panels follow configured bindings. */
export function useShortcutBindings(): ShortcutBindings {
  return useContext(Context)?.bindings ?? defaultShortcuts();
}
