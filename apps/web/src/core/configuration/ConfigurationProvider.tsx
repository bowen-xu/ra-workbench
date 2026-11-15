import {createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, useSyncExternalStore, type PropsWithChildren, type SetStateAction} from "react";
import type {ConfigurationScope, ConfigurationSetting, PluginConfiguration} from "../contracts/configuration";
import {notify} from "../services/notifications";
import {ConfigurationError, freezeConfigurationValue} from "./configuration";

const ConfigurationContext = createContext<PluginConfiguration | undefined>(undefined);

/** App-only binding. Plugins receive the already selected owner, not a factory. */
export function PluginConfigurationProvider({value, children}: PropsWithChildren<{readonly value: PluginConfiguration}>) {
  return <ConfigurationContext.Provider value={value}>{children}</ConfigurationContext.Provider>;
}

export function usePluginConfiguration(): PluginConfiguration {
  const configuration = useContext(ConfigurationContext);
  if (!configuration) throw new ConfigurationError("configuration_unavailable", "unbound");
  return configuration;
}

/** Disabled embedded surfaces retain local presentation state. They neither
 * read nor write the enclosing workspace owner's configuration. */
export function usePluginSetting<T>(setting: ConfigurationSetting<T>, options: {readonly resourceId?: string; readonly enabled?: boolean} = {}) {
  const configuration = useContext(ConfigurationContext), enabled = options.enabled !== false;
  if (enabled && !configuration) throw new ConfigurationError("configuration_unavailable", "unbound", setting.key);
  const fallback = useMemo(() => freezeConfigurationValue(setting.defaultValue), [setting.defaultValue]);
  const identity = JSON.stringify([configuration?.owner, setting.key, options.resourceId]);
  const [local, setLocal] = useState({identity, value: fallback});
  const lastValid = useRef({identity, value: fallback}), readError = useRef<ConfigurationError | undefined>(undefined), reported = useRef<string | undefined>(undefined);
  const reportReadError = useCallback(() => {
    const message = readError.current?.message;
    if (message === reported.current) return;
    reported.current = message;
    if (message) notify.error(message);
  }, []);
  const subscribe = useCallback((changed: () => void) => enabled && configuration ? configuration.subscribe(setting.key, () => {changed(); reportReadError();}) : () => {}, [configuration, enabled, setting.key, reportReadError]);
  const read = useCallback(() => {
    try {
      const value = enabled && configuration ? configuration.get<T>(setting.key, options.resourceId === undefined ? {kind: "application"} : {kind: "resource", resourceId: options.resourceId}) : fallback;
      lastValid.current = {identity, value}; readError.current = undefined; return value;
    } catch (reason) {
      // A denied/unavailable Storage read must not destroy the management UI.
      // Invalid composition/ownership remains a hard boundary failure.
      if (!(reason instanceof ConfigurationError) || reason.code !== "configuration_read_failed") throw reason;
      readError.current = reason;
      return lastValid.current.identity === identity ? lastValid.current.value : fallback;
    }
  }, [configuration, enabled, setting.key, fallback, options.resourceId, identity]);
  const saved = useSyncExternalStore(subscribe, read, read);
  useEffect(reportReadError);
  const value = enabled ? saved : local.identity === identity ? local.value : fallback;
  const setValue = useCallback((update: SetStateAction<T>) => {
    const next = (previous: T) => typeof update === "function" ? (update as (current: T) => T)(previous) : update;
    if (!enabled) { setLocal(previous => ({identity, value: next(previous.identity === identity ? previous.value : fallback)})); return; }
    const scope: ConfigurationScope = options.resourceId === undefined ? {kind: "application"} : {kind: "resource", resourceId: options.resourceId};
    try { if (configuration) configuration.update(setting.key, next(configuration.get<T>(setting.key, scope)), scope); }
    catch (reason) { notify.error(reason instanceof Error ? reason.message : "configuration_update_failed：无法保存配置"); }
  }, [configuration, enabled, identity, options.resourceId, fallback, setting.key]);
  return [value, setValue] as const;
}
