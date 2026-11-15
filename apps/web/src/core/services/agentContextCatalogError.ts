import { nativeDataDiagnostic } from "./nativeDataDiagnostic";

const FALLBACK = "无法读取材料，请重试。";
const MESSAGE = "文献上下文目录来源不可用。";
const RESOURCES = new Set([
  "literature.context-catalog.papers",
  "literature.context-catalog.pdf-attachments",
]);

/** Project only the new finite catalog failure; legacy failures keep their UI. */
export function contextCatalogErrorMessage(reason: unknown): string {
  if (reason === null || typeof reason !== "object" || Array.isArray(reason)) return FALLBACK;
  const value = reason as Record<string, unknown>;
  if (value.message !== MESSAGE || typeof value.code !== "string"
    || !/^(?:plugin_|runtime_plugin_)[a-z0-9_]+$/u.test(value.code)
    || typeof value.phase !== "string"
    || !["admission", "resource", "read"].includes(value.phase)) return FALLBACK;
  const resource = typeof value.resourceId === "string" && RESOURCES.has(value.resourceId);
  const safe = nativeDataDiagnostic({
    ...value,
    resourceId: resource ? value.resourceId : undefined,
    resourceVersion: resource && value.resourceVersion === 1 ? 1 : undefined,
  });
  const fields = Object.entries(safe ?? {}).map(([key, entry]) => `${key}=${entry}`);
  return [MESSAGE, "请核对文献插件后重试。", ...fields].join(" · ");
}
