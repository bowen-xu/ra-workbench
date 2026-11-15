import { getAppStorage } from "../../runtime/appStorage";
import type { LlmModelDescriptor, LlmProviderId } from "@research-assistant/core/ai";
import { decodeLlmProviderModels } from "../../services/aiDecoders";

const CACHE_KEY = "research-assistant:llm-model-catalog:v2";
const SCHEMA_VERSION = 2;

type ModelsByProvider = Partial<Record<LlmProviderId, readonly LlmModelDescriptor[]>>;

interface PersistedCatalog {
  readonly schemaVersion: typeof SCHEMA_VERSION;
  readonly providers: Partial<Record<LlmProviderId, {
    readonly models: readonly LlmModelDescriptor[];
    readonly refreshedAt: string;
  }>>;
}

function isString(value: unknown): value is string {
  return typeof value === "string";
}

function modelBelongsToProvider(providerId: LlmProviderId, model: LlmModelDescriptor): boolean {
  const id = model.id.toLowerCase();
  switch (providerId) {
    case "openai":
      return !id.includes("qwen") && !id.includes("qvq") && !id.includes("deepseek");
    case "qwen":
    case "deepseek":
      return true;
  }
}

function readCatalog(): PersistedCatalog {
  const empty: PersistedCatalog = { schemaVersion: SCHEMA_VERSION, providers: {} };
  try {
    const raw = getAppStorage().getItem(CACHE_KEY);
    if (raw === null) return empty;
    const value: unknown = JSON.parse(raw);
    if (typeof value !== "object" || value === null) return empty;
    const record = value as Record<string, unknown>;
    if (record.schemaVersion !== SCHEMA_VERSION || typeof record.providers !== "object" || record.providers === null) return empty;
    const providers: PersistedCatalog["providers"] = {};
    // ChatGPT caches belong to the authenticated native account scope. A
    // provider-only browser cache must never bypass that identity/TTL check.
    for (const providerId of ["deepseek", "qwen"] as const) {
      const entry = (record.providers as Record<string, unknown>)[providerId];
      if (typeof entry !== "object" || entry === null) continue;
      const candidate = entry as Record<string, unknown>;
      if (!Array.isArray(candidate.models) || !isString(candidate.refreshedAt)) continue;
      const models = decodeLlmProviderModels({ providerId, models: candidate.models })?.models;
      if (models === undefined) continue;
      const compatibleModels = models
        .filter((model) => modelBelongsToProvider(providerId, model));
      if (compatibleModels.length === 0) continue;
      providers[providerId] = {
        models: compatibleModels,
        refreshedAt: candidate.refreshedAt,
      };
    }
    return { schemaVersion: SCHEMA_VERSION, providers };
  } catch {
    return empty;
  }
}

function writeCatalog(catalog: PersistedCatalog): void {
  try {
    getAppStorage().setItem(CACHE_KEY, JSON.stringify(catalog));
  } catch {
    // Model discovery still works when storage is unavailable; only cross-view reuse is lost.
  }
}

export function readCachedLlmModels(providerId: LlmProviderId): readonly LlmModelDescriptor[] | undefined {
  return readCatalog().providers[providerId]?.models;
}

export function readAllCachedLlmModels(): ModelsByProvider {
  const catalog = readCatalog();
  return Object.fromEntries(Object.entries(catalog.providers).map(([providerId, entry]) => [providerId, entry?.models])) as ModelsByProvider;
}

export function cacheLlmModels(providerId: LlmProviderId, models: readonly LlmModelDescriptor[]): void {
  if (providerId === "openai") {
    removeCachedLlmModels(providerId);
    return;
  }
  const current = readCatalog();
  const compatibleModels = models.filter((model) => modelBelongsToProvider(providerId, model));
  if (compatibleModels.length === 0) {
    const { [providerId]: _removed, ...providers } = current.providers;
    writeCatalog({ schemaVersion: SCHEMA_VERSION, providers });
    return;
  }
  writeCatalog({
    schemaVersion: SCHEMA_VERSION,
    providers: {
      ...current.providers,
      [providerId]: { models: compatibleModels, refreshedAt: new Date().toISOString() },
    },
  });
}

export function removeCachedLlmModels(providerId: LlmProviderId): void {
  const current = readCatalog();
  const { [providerId]: _removed, ...providers } = current.providers;
  writeCatalog({ schemaVersion: SCHEMA_VERSION, providers });
}
