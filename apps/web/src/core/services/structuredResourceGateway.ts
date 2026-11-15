import { listen } from "@tauri-apps/api/event";
import type { AiEvidenceItem } from "@research-assistant/core/ai";
import { decodeEvidenceItem } from "./aiCandidateDecoders";
import { getAppStorage } from "../runtime/appStorage";
import { decodeStructuredResource, decodeStructuredResourceList, decodeStructuredWriteResult, type StructuredLegacyImport, type StructuredResource, type StructuredResourceWrite, type StructuredReviewRequest, type StructuredTemplateBinding, type StructuredWriteResult, } from "@research-assistant/core/structured-resources";
import { getAppRuntime, resolveAppGateway } from "../runtime/appRuntime";
import { invoke, isDesktopRuntime } from "../runtime/platformCore";
import { structuredSourceError } from "./structuredSourceError";
import { hbsDocumentError, hbsNativeRejectionIsDefinite } from "./hbsNativeFailure";
import { nativeDiagnosticIdentity, nativeDiagnosticPackageKey } from "./nativeDataDiagnostic";
import type { StructuredStartupImport, StructuredStartupImportResult } from "@research-assistant/core/structured-resources";
import type { StructuredHistoryRequest, StructuredValidationOrigin } from "@research-assistant/core/structured-resources";
export type StructuredResourceEvidence = AiEvidenceItem & {
  readonly fieldIds?: readonly string[];
};
export interface StructuredResourceGateway {
  subscribeChanges?(changed: (change: {
    resourceId: string;
    paperId: string | null;
  }) => void): Promise<() => void>;
  createTemplate(title: string, requestId: string): Promise<StructuredResource>;
  list(paperId?: string): Promise<readonly StructuredResource[]>;
  get(resourceId: string): Promise<StructuredResource>;
  getSources(resourceId: string): Promise<readonly StructuredResourceEvidence[]>;
  getVersion(resourceId: string, revision: number): Promise<StructuredResource>;
  write(draft: StructuredResourceWrite): Promise<StructuredWriteResult>;
  review(draft: StructuredReviewRequest): Promise<StructuredResource>;
  restoreHistory(draft: StructuredHistoryRequest): Promise<StructuredResource>;
  bindTemplate(draft: StructuredTemplateBinding): Promise<StructuredResource>;
  importLegacy(draft: StructuredLegacyImport): Promise<readonly StructuredResource[]>;
  importStartup(draft: StructuredStartupImport): Promise<StructuredStartupImportResult>;
}
function usesNativeMutationDiagnostics(command: string): boolean {
  return command === "structured_review_resource" || command === "structured_restore_resource_history" || command === "structured_write_resource";
}
async function call(command: string, args: Record<string, unknown>): Promise<unknown> {
  const runtime = getAppRuntime();
  try {
    if (runtime.mode === "preview") {
      if (!runtime.invoke)
        throw new Error("预览缺少结构化资源适配器");
      return await runtime.invoke(command, args);
    }
    if (!isDesktopRuntime())
      throw new Error("结构化资源需要桌面应用");
    return await invoke(command, args);
  }
  catch (reason) {
    if (usesNativeMutationDiagnostics(command) && reason && typeof reason === "object") {
      if (command === "structured_write_resource" && !("message" in reason && typeof reason.message === "string"))
        throw hbsDocumentError({code: "code" in reason ? reason.code : undefined,
          message: "结构化资源写入失败，请重试原请求核对草稿。", nativeFailure: undefined});
      if (Object.hasOwn(reason, "nativeFailure")) throw hbsDocumentError(reason);
      const prototype = Object.getPrototypeOf(reason), keys = Reflect.ownKeys(reason);
      const value = reason as Record<string, unknown>;
      const code = nativeDiagnosticIdentity(value.code), requestId = nativeDiagnosticIdentity(value.requestId);
      const dispatchedId = args.draft && typeof args.draft === "object" && "requestId" in args.draft ? args.draft.requestId : undefined;
      if ((prototype === Object.prototype || prototype === null) && keys.length === 3
        && ["code", "message", "requestId"].every(key => keys.includes(key))
        && code !== undefined && requestId !== undefined && typeof value.message === "string"
        && (command !== "structured_write_resource" || requestId === dispatchedId))
        throw hbsDocumentError({code, message: `${value.message} · requestId=${requestId}`, requestId});
      // Only the original two-field static envelope is an unmarked write
      // failure. A damaged/technical reply must retain the immutable request.
      if (command === "structured_write_resource" && typeof value.message === "string"
        && !((prototype === Object.prototype || prototype === null) && keys.length === 2
          && keys.includes("code") && keys.includes("message") && typeof value.code === "string"))
        throw hbsDocumentError({code: value.code, message: value.message, requestId: value.requestId, nativeFailure: undefined});
    }
    if (["structured_resource_import_startup", "structured_import_legacy", "structured_get_resource_sources"].includes(command)) {
      const error = structuredSourceError(reason);
      if (error) throw error;
    }
    if (reason && typeof reason === "object" && "message" in reason && typeof reason.message === "string" && "code" in reason && typeof reason.code === "string")
      throw Object.assign(new Error(reason.message), { code: reason.code });
    throw reason;
  }
}
function safeValidationOrigin(value: unknown): StructuredValidationOrigin | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) return undefined;
  const origin = value as Record<string, unknown>, keys = Reflect.ownKeys(origin), prototype = Object.getPrototypeOf(origin);
  if ((prototype !== Object.prototype && prototype !== null) || keys.length !== 4
    || !["requestId", "packageKey", "instanceId", "nativeRequestId"].every(key => Object.hasOwn(origin, key))) return undefined;
  const coreId = nativeDiagnosticIdentity(origin.requestId), packageKey = nativeDiagnosticPackageKey(origin.packageKey);
  const instanceId = nativeDiagnosticIdentity(origin.instanceId), nativeRequestId = nativeDiagnosticIdentity(origin.nativeRequestId);
  if (coreId === undefined || packageKey === undefined || instanceId === undefined || nativeRequestId === undefined) return undefined;
  return {requestId: coreId, packageKey, instanceId, nativeRequestId};
}
/** Safe suffix for the existing validation error surface, including typed
 * preview ports. Dispatch binding remains the gateway's responsibility. */
export function structuredWriteValidationDiagnostic(result: StructuredWriteResult): string {
  const origin = result.status === "invalid" ? safeValidationOrigin(result.validationOrigin) : undefined;
  return origin ? Object.entries(origin).map(([key, value]) => `${key}=${value}`).join(" · ") : "";
}
function acknowledgedWriteResult(value: unknown, requestId: string): StructuredWriteResult {
  if (value && typeof value === "object" && !Array.isArray(value) && Object.hasOwn(value, "validationOrigin")) {
    const {validationOrigin, ...original} = value as Record<string, unknown>;
    const origin = original.status === "invalid" ? safeValidationOrigin(validationOrigin) : undefined;
    // Optional diagnostics cannot hide an otherwise valid, acknowledged
    // domain response. The public DTO decoder remains strict for direct use.
    return decodeStructuredWriteResult({...original, ...(origin?.requestId === requestId ? {validationOrigin: origin} : {})});
  }
  return decodeStructuredWriteResult(value);
}
/** Retain dispatch identity until acknowledged, so a retry settles the original write. */
async function mutate<T extends {
  readonly requestId: string;
}>(command: string, draft: T, target: string, flat = false): Promise<unknown> {
  const storage = getAppStorage(), key = `research-assistant:structured-request:${command}:${target}`;
  const dispatch = async (args: Record<string, unknown>, requestId: string): Promise<unknown> => {
    try {
      return await call(command, args);
    }
    catch (reason) {
      if ((!usesNativeMutationDiagnostics(command) || hbsNativeRejectionIsDefinite(reason, requestId) !== false)
        && reason && typeof reason === "object" && "code" in reason && ["revision_conflict", "invalid_input", "not_found"].includes(String(reason.code)))
        storage.removeItem(key);
      throw reason;
    }
  };
  const pending = storage.getItem(key);
  if (pending) {
    const previous = JSON.parse(pending) as Record<string, unknown>;
    const result = await dispatch(flat ? previous : { draft: previous }, typeof previous.requestId === "string" ? previous.requestId : "");
    storage.removeItem(key);
    const withoutId = (value: object): string => JSON.stringify(Object.fromEntries(Object.entries(value).filter(([name]) => name !== "requestId")));
    if (withoutId(previous) === withoutId(draft))
      return command === "structured_write_resource" ? acknowledgedWriteResult(result, typeof previous.requestId === "string" ? previous.requestId : "") : result;
    throw new Error("上次操作已完成核对，请刷新内容后重新提交当前修改。");
  }
  storage.setItem(key, JSON.stringify(draft));
  const result = await dispatch(flat ? draft : { draft }, draft.requestId);
  storage.removeItem(key);
  return command === "structured_write_resource" ? acknowledgedWriteResult(result, draft.requestId) : result;
}
const native: StructuredResourceGateway = {
  subscribeChanges: async (changed) => listen<{
    resourceId: string;
    paperId: string | null;
  }>("research-assistant:structured-resource-changed", event => changed(event.payload)),
  createTemplate: async (title, requestId) => decodeStructuredResource(await mutate("structured_create_template", { title, requestId }, "new-template", true)),
  list: async (paperId) => decodeStructuredResourceList(await call("structured_list_resources", { paperId: paperId ?? null })),
  getSources: async (resourceId) => {
    const raw = await call("structured_get_resource_sources", { resourceId });
    if (!Array.isArray(raw))
      throw new Error("资源来源回执无效");
    return raw.map(value => {
      const evidence = decodeEvidenceItem(value);
      if (!evidence)
        throw new Error("资源来源无效");
      const fieldIds = (value as {
        fieldIds?: unknown;
      }).fieldIds;
      if (fieldIds !== undefined && (!Array.isArray(fieldIds) || fieldIds.some(id => typeof id !== "string")))
        throw new Error("资源来源字段归属无效");
      return { ...evidence, ...(fieldIds === undefined ? {} : { fieldIds: fieldIds as string[] }) };
    });
  },
  getVersion: async (resourceId, revision) => decodeStructuredResource(await call("structured_get_resource_version", { resourceId, revision })),
  get: async (resourceId) => decodeStructuredResource(await call("structured_get_resource", { resourceId })),
  write: async (draft) => decodeStructuredWriteResult(await mutate("structured_write_resource", draft, draft.resourceId)),
  review: async (draft) => decodeStructuredResource(await mutate("structured_review_resource", draft, draft.resourceId)),
  restoreHistory: async draft => decodeStructuredResource(await mutate("structured_restore_resource_history", draft, draft.resourceId)),
  bindTemplate: async (draft) => decodeStructuredResource(await mutate("structured_bind_template", draft, draft.paperId)),
  importLegacy: async (draft) => decodeStructuredResourceList(await call("structured_import_legacy", { draft })),
  importStartup: async draft => {
    const value = await call("structured_resource_import_startup", {draft});
    if (!value || typeof value !== "object" || !("paperIds" in value) || !Array.isArray(value.paperIds) ||
      value.paperIds.some(id => typeof id !== "string") || !("importedPaperCount" in value) ||
      !("importedTemplateCount" in value) || !Number.isSafeInteger(value.importedPaperCount) ||
      !Number.isSafeInteger(value.importedTemplateCount)) throw new Error("启动迁移回执无效");
    for (const key of ["deferredPaperIds", "deferredContextIds"] as const) {
      const ids = (value as Record<string, unknown>)[key];
      if (ids !== undefined && (!Array.isArray(ids) || ids.some(id => typeof id !== "string")))
        throw new Error("启动迁移回执无效");
    }
    return value as StructuredStartupImportResult;
  },
};
export function getStructuredResourceGateway(): StructuredResourceGateway {
  return resolveAppGateway("getStructuredResourceGateway", native);
}
