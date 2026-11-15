import {useCallback, useEffect, useMemo, useState, type SetStateAction} from "react";
import {getAppStorage} from "./appStorage";
import {notify} from "../services/notifications";

const prefix = "research-assistant:workspace.v1:";
export function readWorkspaceState<T>(key: string, fallback: T, decode: (value: unknown) => T): T {
  try {
    const raw = getAppStorage().getItem(prefix + key);
    return raw === null ? fallback : decode(JSON.parse(raw));
  } catch { return fallback; }
}
export function writeWorkspaceState(key: string, value: unknown): void {
  try {
    if (value === undefined) { getAppStorage().removeItem(prefix + key); return; }
    const storage = getAppStorage(), raw = JSON.stringify(value, (_key, item) => item instanceof Set ? [...item] : item);
    if (storage.getItem(prefix + key) !== raw) storage.setItem(prefix + key, raw);
  } catch (reason) {
    notify.error(`workspace_state_write_failed：无法保存工作区状态（${key}）：${reason instanceof Error ? reason.message : String(reason)}`);
  }
}

/** Presentation only; documents, permissions and review state retain their
 * domain owners. A changed key reads its own state before any write occurs. */
export function useWorkspaceState<T>(key: string, fallback: T, decode: (value: unknown) => T, enabled = true) {
  const initial = useMemo(() => enabled ? readWorkspaceState(key, fallback, decode) : fallback, [key, enabled]);
  const [state, setState] = useState(() => ({key, value:initial}));
  const value = state.key === key ? state.value : initial;
  const setValue = useCallback((update: SetStateAction<T>) => setState(previous => {
    const current = previous.key === key ? previous.value : initial;
    return {key, value: typeof update === "function" ? (update as (value:T)=>T)(current) : update};
  }), [key, initial]);
  useEffect(() => { if (enabled) writeWorkspaceState(key,value); }, [key,value,enabled]);
  return [value,setValue] as const;
}
export const workspaceBoolean = (value: unknown): boolean => value === true;
export const workspaceString = (value: unknown): string => typeof value === "string" ? value : "";
export const workspaceStrings = (value: unknown): readonly string[] => Array.isArray(value)
  ? [...new Set(value.filter((id): id is string => typeof id === "string" && id.length > 0 && id.length <= 4096))].slice(0,256) : [];
export const workspaceStringSet = (value: unknown): ReadonlySet<string> => new Set(workspaceStrings(value));
