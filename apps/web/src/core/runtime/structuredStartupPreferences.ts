import { DEFAULT_PORTRAIT_CATALOG, decodePortraitCatalog } from "@research-assistant/core/ai";
import type { StructuredLegacyImport, StructuredStartupImport } from "@research-assistant/core/structured-resources";

const PRESENTATION_PREFIX = "research-assistant.reading-presentation.v1:";
const CONTEXT_PREFIX = "research-assistant:agent-context:v1:";
const CATALOG_KEY = "research-assistant.portrait-catalog.v1";
const identity = (value: unknown): value is string => typeof value === "string" &&
  value.trim() === value && value.length > 0 && value.length <= 1024 && !/[\u0000-\u001f\u007f/\\]/u.test(value);
const record = (value: unknown): Record<string, unknown> => {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("需要配置对象");
  return value as Record<string, unknown>;
};

function presentation(raw: string): NonNullable<StructuredLegacyImport["presentation"]> {
  const value = record(JSON.parse(raw));
  if (Object.keys(value).some(key => !["fields", "hiddenUnitIds", "hiddenResourceVersions"].includes(key)) ||
    !Array.isArray(value.fields) || value.fields.length > 128 || !Array.isArray(value.hiddenUnitIds) ||
    value.hiddenUnitIds.some(id => !identity(id))) throw new Error("阅读显示配置格式无效");
  const fields = value.fields.map(entry => {
    const row = record(entry);
    if (Object.keys(row).some(key => !["candidateId", "fieldId"].includes(key)) ||
      !identity(row.candidateId) || !identity(row.fieldId)) throw new Error("阅读字段选择无效");
    return {candidateId: row.candidateId, fieldId: row.fieldId};
  });
  if (new Set(fields.map(field => field.fieldId)).size !== fields.length) throw new Error("阅读字段选择重复");
  if (value.hiddenResourceVersions !== undefined) {
    if (!Array.isArray(value.hiddenResourceVersions)) throw new Error("隐藏版本配置无效");
    for (const entry of value.hiddenResourceVersions) {
      const row = record(entry);
      if (Object.keys(row).some(key => !["resourceId", "revision"].includes(key)) ||
        !identity(row.resourceId) || typeof row.revision !== "number" || !Number.isSafeInteger(row.revision) ||
        row.revision < 0) throw new Error("隐藏版本配置无效");
    }
  }
  return {fields, hiddenUnitIds: value.hiddenUnitIds as string[]};
}

/** Validate the complete client snapshot before dispatching any native import. */
export function readStructuredStartupPreferences(storage: Storage): StructuredStartupImport {
  const presentations: StructuredStartupImport["presentations"][number][] = [];
  const contexts: StructuredStartupImport["contexts"][number][] = [];
  const keys = Array.from({length: storage.length}, (_, index) => storage.key(index)).filter((key): key is string => key !== null).sort();
  for (const key of keys) {
    if (!key.startsWith(PRESENTATION_PREFIX) && !key.startsWith(CONTEXT_PREFIX)) continue;
    try {
      const raw = storage.getItem(key);
      if (raw === null) throw new Error("配置在读取时发生变化，请重试");
      if (key.startsWith(PRESENTATION_PREFIX)) {
        const paperId = key.slice(PRESENTATION_PREFIX.length);
        if (!identity(paperId)) throw new Error("文献配置身份无效");
        presentations.push({paperId, presentation: presentation(raw)});
      } else {
        const conversationId = key.slice(CONTEXT_PREFIX.length);
        if (!identity(conversationId)) throw new Error("对话配置身份无效");
        // An existing native row (including explicitly empty) wins over even a
        // corrupt cache. The host checks that authority before strict decoding.
        let state: unknown;
        try { state = JSON.parse(raw); } catch { state = raw; }
        contexts.push({conversationId, state});
      }
    } catch (reason) {
      throw new Error(`无法迁移旧配置 ${key}：${reason instanceof Error ? reason.message : "配置无效"}。原配置已保留，请恢复有效配置后重试。`);
    }
  }
  try {
    const raw = storage.getItem(CATALOG_KEY);
    const catalog = raw === null ? DEFAULT_PORTRAIT_CATALOG : decodePortraitCatalog(JSON.parse(raw));
    return {catalog, presentations, contexts};
  } catch {
    throw new Error(`无法迁移旧模板配置 ${CATALOG_KEY}。原配置已保留，请恢复有效配置后重试。`);
  }
}
