import type { ConfigurationInspection, ConfigurationSchema, ConfigurationScope, PluginConfiguration, PluginConfigurationDeclaration } from "../contracts/configuration";
import {appStorageScopedKey} from "../runtime/appStorage";

/** Only App composition supplies these mappings. Plugin declarations cannot
 * choose storage keys or point an otherwise valid setting at another owner. */
export interface ConfigurationRegistration {
  readonly owner: string;
  readonly declaration: PluginConfigurationDeclaration;
  readonly storage: Readonly<Record<string, {readonly application?: string; readonly resourcePrefix?: string}>>;
}

export class ConfigurationError extends Error {
  readonly safeCause?: {readonly name: string; readonly code?: number};
  constructor(readonly code: string, readonly owner: string, readonly setting?: string, reason?: unknown) {
    const safeCause = typeof DOMException !== "undefined" && reason instanceof DOMException &&
      ["QuotaExceededError", "SecurityError", "InvalidStateError", "UnknownError"].includes(reason.name)
      ? Object.freeze({name: reason.name, ...(Number.isSafeInteger(reason.code) ? {code: reason.code} : {})}) : undefined;
    super(`${code}：配置 ${owner}${setting === undefined ? "" : `.${setting}`} 不可用或不符合声明${safeCause ? ` [cause=${safeCause.name}${safeCause.code === undefined ? "" : `; code=${safeCause.code}`}]` : ""}`);
    this.name = "ConfigurationError";
    if (safeCause) this.safeCause = safeCause;
  }
}

function valid(schema: ConfigurationSchema, value: unknown): boolean {
  if (schema.type === "boolean") return typeof value === "boolean";
  if (schema.type === "string") return typeof value === "string" && schema.enum.includes(value);
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const prototype = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) return false;
  const row = value as Record<string, unknown>, properties = Object.entries(schema.properties);
  return Object.keys(row).length === properties.length && properties.every(([key, child]) => Object.hasOwn(row, key) && valid(child, row[key]));
}

export function freezeConfigurationValue<T>(value: T): T {
  if (typeof value !== "object" || value === null) return value;
  if (Array.isArray(value)) return Object.freeze(value.map(freezeConfigurationValue)) as T;
  const copy = Object.fromEntries(Object.entries(value).map(([key, item]) => [key, freezeConfigurationValue(item)]));
  return Object.freeze(copy) as T;
}

function schemaValid(schema: ConfigurationSchema, depth = 0): boolean {
  if (depth > 4 || typeof schema !== "object" || schema === null) return false;
  if (schema.type === "boolean") return true;
  if (schema.type === "string") return Array.isArray(schema.enum) && schema.enum.length > 0 && schema.enum.length <= 64 && new Set(schema.enum).size === schema.enum.length
    && schema.enum.every(value => typeof value === "string" && value.length <= 256);
  if (schema.type !== "object" || !schema.properties || Object.keys(schema.properties).length > 32) return false;
  return Object.entries(schema.properties).every(([key, child]) => /^[a-zA-Z][a-zA-Z0-9]*$/.test(key) && schemaValid(child, depth + 1));
}

/** App-only constructor. It adds no persistent store: mappings address the
 * exact existing workbench Storage facts, captured from the selected runtime. */
export function createPluginConfigurations(storage: Storage, registrations: readonly ConfigurationRegistration[]): ReadonlyMap<string, PluginConfiguration> {
  const owners = new Map<string, PluginConfiguration>(), claimed: {key: string; prefix: boolean}[] = [];
  for (const registration of registrations) {
    const {declaration} = registration;
    if (registration.owner !== declaration.owner) throw new ConfigurationError("configuration_owner_mismatch", registration.owner);
    if (!/^[a-z][a-z0-9_]*$/.test(declaration.owner) || owners.has(declaration.owner)) throw new ConfigurationError("configuration_owner_conflict", declaration.owner);
    const settings = new Map<string, (typeof declaration.settings)[number]>();
    for (const setting of declaration.settings) {
      const mapping = registration.storage[setting.key];
      if (!/^[a-z][a-zA-Z0-9]*$/.test(setting.key) || settings.has(setting.key)) throw new ConfigurationError("configuration_key_conflict", declaration.owner, setting.key);
      if (!schemaValid(setting.schema) || !valid(setting.schema, setting.defaultValue) || setting.effect !== "immediate" || !setting.scopes.length
        || new Set(setting.scopes).size !== setting.scopes.length || setting.scopes.some(scope => scope !== "application" && scope !== "resource")
        || !mapping || setting.scopes.some(scope => scope === "application" ? !mapping.application : !mapping.resourcePrefix)
        || (mapping.application !== undefined && !setting.scopes.includes("application")) || (mapping.resourcePrefix !== undefined && !setting.scopes.includes("resource"))) {
        throw new ConfigurationError("configuration_declaration_invalid", declaration.owner, setting.key);
      }
      for (const [key, prefix] of [[mapping.application, false], [mapping.resourcePrefix, true]] as const) {
        if (key === undefined) continue;
        if (!key || claimed.some(prior => prior.key === key || (prior.prefix && key.startsWith(prior.key)) || (prefix && prior.key.startsWith(key)))) {
          throw new ConfigurationError("configuration_storage_conflict", declaration.owner, setting.key);
        }
        claimed.push({key, prefix});
      }
      settings.set(setting.key, {...setting, schema: freezeConfigurationValue(setting.schema), scopes: Object.freeze([...setting.scopes]), defaultValue: freezeConfigurationValue(setting.defaultValue)});
    }
    if (Object.keys(registration.storage).some(key => !settings.has(key))) throw new ConfigurationError("configuration_declaration_invalid", declaration.owner);
    const mappings = Object.fromEntries(Object.entries(registration.storage).map(([key, mapping]) => [key, Object.freeze({...mapping})]));
    owners.set(declaration.owner, bindConfiguration(storage, {...registration, storage: Object.freeze(mappings)}, settings));
  }
  return owners;
}

function bindConfiguration(storage: Storage, registration: ConfigurationRegistration, settings: ReadonlyMap<string, PluginConfigurationDeclaration["settings"][number]>): PluginConfiguration {
  const owner = registration.declaration.owner;
  const listeners = new Map<string, Set<() => void>>(), cache = new Map<string, {raw: string; inspection: ConfigurationInspection<unknown>}>();
  const requireSetting = (key: string) => {
    const setting = settings.get(key);
    if (!setting) throw new ConfigurationError("configuration_not_authorized", owner, key);
    return setting;
  };
  const resource = (key: string, scope?: ConfigurationScope) => {
    const setting = requireSetting(key);
    if (scope !== undefined && scope.kind !== "application" && scope.kind !== "resource") throw new ConfigurationError("configuration_scope_invalid", owner, key);
    if (scope?.kind === "resource" && (!setting.scopes.includes("resource") || !scope.resourceId || scope.resourceId.length > 4096 || /[\u0000-\u001f]/.test(scope.resourceId))) {
      throw new ConfigurationError("configuration_scope_invalid", owner, key);
    }
    return scope?.kind === "resource" ? scope.resourceId : undefined;
  };
  const requireMapping = (key: string) => {
    const mapping = registration.storage[key];
    if (!mapping) throw new ConfigurationError("configuration_not_authorized", owner, key);
    return mapping;
  };
  const emit = (key: string) => { for (const changed of [...(listeners.get(key) ?? [])]) changed(); };
  const onStorage = (event: StorageEvent) => {
    for (const [key, mapping] of Object.entries(registration.storage)) {
      if (event.key === null || (mapping.application !== undefined && event.key === appStorageScopedKey(storage, mapping.application))
        || (mapping.resourcePrefix !== undefined && event.key?.startsWith(appStorageScopedKey(storage, mapping.resourcePrefix)))) emit(key);
    }
  };
  function inspect<T>(key: string, scope?: ConfigurationScope): ConfigurationInspection<T> {
    const setting = requireSetting(key), resourceId = resource(key, scope), mapping = requireMapping(key);
    let applicationRaw: string | null, resourceRaw: string | null;
    try {
      applicationRaw = mapping.application === undefined ? null : storage.getItem(mapping.application);
      resourceRaw = resourceId === undefined || mapping.resourcePrefix === undefined ? null : storage.getItem(mapping.resourcePrefix + resourceId);
    } catch (reason) { throw new ConfigurationError("configuration_read_failed", owner, key, reason); }
    const cacheKey = JSON.stringify([key, resourceId]), raw = JSON.stringify([applicationRaw, resourceRaw]);
    const previous = cache.get(cacheKey);
    if (previous?.raw === raw) return previous.inspection as ConfigurationInspection<T>;
    const invalidScopes: ConfigurationScope["kind"][] = [];
    function read(rawValue: string | null, tier: ConfigurationScope["kind"]): unknown {
      if (rawValue === null) return undefined;
      try { const value: unknown = JSON.parse(rawValue); if (valid(setting.schema, value)) return freezeConfigurationValue(value); } catch { /* Historical invalid state retains its bytes. */ }
      invalidScopes.push(tier); return undefined;
    }
    const applicationValue = read(applicationRaw, "application"), resourceValue = read(resourceRaw, "resource");
    const inspection = Object.freeze({defaultValue: setting.defaultValue, ...(applicationValue === undefined ? {} : {applicationValue}),
      ...(resourceValue === undefined ? {} : {resourceValue}), value: resourceValue ?? applicationValue ?? setting.defaultValue,
      origin: resourceValue !== undefined ? "resource" : applicationValue !== undefined ? "application" : "default", invalidScopes: Object.freeze(invalidScopes)} as const);
    cache.set(cacheKey, {raw, inspection});
    return inspection as ConfigurationInspection<T>;
  }
  return Object.freeze({owner, inspect, get: <T,>(key: string, scope?: ConfigurationScope) => inspect<T>(key, scope).value,
    update(key: string, value: unknown, scope: ConfigurationScope = {kind: "application"}) {
      const setting = requireSetting(key), resourceId = resource(key, scope), mapping = requireMapping(key);
      if (!setting.scopes.includes(scope.kind)) throw new ConfigurationError("configuration_scope_invalid", owner, key);
      // Persist the validated data snapshot, not caller-owned serialization
      // methods that can encode a different value than the schema accepted.
      let snapshot: unknown;
      try {
        if (value !== undefined && !valid(setting.schema, value)) throw new Error("invalid");
        snapshot = value === undefined ? undefined : freezeConfigurationValue(value);
        if (snapshot !== undefined && !valid(setting.schema, snapshot)) throw new Error("invalid");
      } catch { throw new ConfigurationError("configuration_value_invalid", owner, key); }
      const storageKey = scope.kind === "resource" && mapping.resourcePrefix !== undefined && resourceId !== undefined ? mapping.resourcePrefix + resourceId : mapping.application;
      if (storageKey === undefined) throw new ConfigurationError("configuration_scope_invalid", owner, key);
      try {
        const before = storage.getItem(storageKey), after = snapshot === undefined ? null : JSON.stringify(snapshot);
        if (before === after) return;
        if (after === null) storage.removeItem(storageKey); else storage.setItem(storageKey, after);
      } catch (reason) { throw new ConfigurationError("configuration_write_failed", owner, key, reason); }
      emit(key);
    },
    subscribe(key: string, changed: () => void) {
      requireSetting(key);
      if (![...listeners.values()].some(group => group.size > 0) && typeof window !== "undefined") window.addEventListener("storage", onStorage);
      const group = listeners.get(key) ?? new Set<() => void>(), listener = () => changed(); group.add(listener); listeners.set(key, group);
      return () => {
        group.delete(listener);
        if (![...listeners.values()].some(item => item.size > 0) && typeof window !== "undefined") window.removeEventListener("storage", onStorage);
      };
    },
  });
}
