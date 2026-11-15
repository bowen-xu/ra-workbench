import type {ManuscriptImportPreview} from "@research-assistant/writing";
import {hasPluginFrontend, PluginPackageError, WORKBENCH_NATIVE_PROTOCOL_VERSION} from "@research-assistant/workbench-sdk";
import {coreCapabilities} from "../../coreCapabilities";
import type {NativeTerminationDiagnostic} from "../../../core/public/diagnostics";
import {diagnosticError, nativeTerminationDiagnostic, nativeTerminationMessage} from "../../../core/public/diagnostics";
import {invoke} from "../../../core/runtime/platformCore";
import {resolveRuntimePluginDiscovery, RuntimePluginLoadError} from "../../../core/runtime/plugins/loader";

const PLUGIN_ID = "writing_projects";

interface PreviewDiagnostic {
  readonly code: string;
  readonly message: string;
  readonly pluginId: string;
  readonly packageKey?: string;
  readonly instanceId?: string;
  readonly requestId?: string;
  readonly exitCode?: number;
  readonly signal?: number;
  readonly cleanupError?: string;
  readonly terminationError?: NativeTerminationDiagnostic;
  readonly currentRevision?: number;
  readonly operationId?: string;
  readonly status?: number;
  readonly details?: readonly string[];
}

/** The original import UI displays message only. Preserve safe native identity
 * there as well as structured fields and the untouched original rejection. */
class NativeWritingPreviewError extends Error {
  readonly code: string;
  readonly pluginId: string;
  readonly packageKey?: string;
  readonly instanceId?: string;
  readonly requestId?: string;
  readonly exitCode?: number;
  readonly signal?: number;
  readonly cleanupError?: string;
  readonly terminationError?: NativeTerminationDiagnostic;

  constructor(readonly diagnostic: PreviewDiagnostic, cause?: unknown) {
    const {message, details, ...fields} = diagnostic;
    const terminationMessage = nativeTerminationMessage(diagnostic.terminationError);
    super([`${diagnostic.code}: ${message}`, ...Object.entries(fields)
      .filter(([field]) => field !== "code" && field !== "terminationError")
      .map(([field, value]) => `${field}=${value}`),
      ...(details ?? []), ...(terminationMessage === undefined ? [] : [terminationMessage])].join(" · "), {cause});
    this.name = "NativeWritingPreviewError";
    this.code = diagnostic.code;
    this.pluginId = diagnostic.pluginId;
    Object.assign(this, fields);
  }
}

function record(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : undefined;
}

function enabled(discovered: unknown): boolean {
  const envelope = record(discovered);
  for (const [field, valid, expected] of [
    ["discovery", envelope !== undefined, "object"],
    ["nativeWritingPreview", typeof envelope?.nativeWritingPreview === "boolean", "boolean"],
    ["packages", Array.isArray(envelope?.packages), "array"],
  ] as const) {
    if (!valid) throw new RuntimePluginLoadError({code: "plugin_discovery_invalid", pluginId: "<discovery>",
      field, expected, actual: field === "discovery" ? typeof discovered : typeof envelope?.[field]});
  }
  return envelope?.nativeWritingPreview === true;
}

function safeIdentity(value: unknown): string | undefined {
  return typeof value === "string" && /^[\w:.-]{1,200}$/u.test(value) ? value : undefined;
}

function nativeFailure(reason: unknown, fallbackCode: string, packageKey?: string, profile: "preview" | "projects" = "preview"): NativeWritingPreviewError {
  // Reuse the shared diagnostic categorization and request-ID policy. These
  // three additional fields belong to the finite native host contract.
  const safe = diagnosticError(reason);
  const original = record(reason);
  const code = typeof safe.code === "string" ? safe.code : fallbackCode;
  const knownNativeCode = typeof safe.code === "string"
    && (/^(?:(?:plugin_host|runtime_plugin)_[a-z_]+|invalid_input|unsafe_path|file_too_large|storage_error)$/u.test(safe.code)
      || profile === "projects" && (/^plugin_data_[a-z_]+$/u.test(safe.code)
        || ["not_found", "managed_bibliography", "compile_already_running", "compile_run_not_found", "invalid_engine", "toolchain_unavailable", "synctex_unavailable", "synctex_no_result", "compile_process_error", "checkpoint_conflict", "file_conflict", "revision_conflict", "index_failed", "unsupported_encoding", "invalid_bibtex_entry", "bibliography_import", "bibliography_conflict", "literature_citations_unavailable", "citation_source_not_found"].includes(safe.code)));
  const message = knownNativeCode && typeof original?.message === "string"
    ? original.message : typeof safe.cause === "string" ? safe.cause : "unclassified";
  const actualPackageKey = safeIdentity(original?.packageKey) ?? packageKey;
  const instanceId = safeIdentity(original?.instanceId);
  const operationId = safeIdentity(original?.operationId);
  const cleanupError = knownNativeCode && typeof original?.cleanupError === "string" ? original.cleanupError : undefined;
  const terminationError = knownNativeCode ? nativeTerminationDiagnostic(original?.terminationError) : undefined;
  return new NativeWritingPreviewError({code, message,
    pluginId: safeIdentity(original?.pluginId) ?? PLUGIN_ID,
    ...(actualPackageKey ? {packageKey: actualPackageKey} : {}),
    ...(instanceId ? {instanceId} : {}),
    ...(typeof safe.requestId === "string" ? {requestId: safe.requestId} : {}),
    ...(Number.isSafeInteger(original?.exitCode) ? {exitCode: original?.exitCode as number} : {}),
    ...(Number.isSafeInteger(original?.signal) ? {signal: original?.signal as number} : {}),
    ...(cleanupError === undefined ? {} : {cleanupError}),
    ...(terminationError === undefined ? {} : {terminationError}),
    ...(profile === "projects" ? {
      ...(operationId ? {operationId} : {}),
      ...(Number.isInteger(safe.httpStatus) && Number(safe.httpStatus) >= 100 && Number(safe.httpStatus) <= 599 ? {status: safe.httpStatus as number} : {}),
      ...(Number.isSafeInteger(original?.currentRevision) ? {currentRevision: original?.currentRevision as number} : {}),
      details: projectDiagnosticFields(original),
    } : {}),
  }, reason);
}

function projectDiagnosticFields(value: Record<string, unknown> | undefined): readonly string[] {
  const fields: string[] = [];
  for (const slot of ["dataError", "saveDatabaseError", "saveRollbackError", "queryDatabaseError", "queryRollbackError"] as const) {
    const source = record(value?.[slot]);
    if (!source) continue;
    for (const key of ["code", "phase", "safeCause", "sqliteCode", "resourceId"] as const) {
      const token = source[key];
      if (typeof token === "string" && /^[a-zA-Z][a-zA-Z0-9_.-]{0,159}$/u.test(token)) fields.push(`${slot}.${key}=${token}`);
    }
    if (Number.isInteger(source.sqliteExtendedCode) && Number(source.sqliteExtendedCode) >= -2_147_483_648
      && Number(source.sqliteExtendedCode) <= 2_147_483_647) fields.push(`${slot}.sqliteExtendedCode=${source.sqliteExtendedCode}`);
  }
  if (typeof value?.saveOutcome === "string" && ["notStarted", "rolledBack", "committed", "unknown"].includes(value.saveOutcome)) fields.push(`saveOutcome=${value?.saveOutcome}`);
  return fields;
}

/** Reuse the preview's safe native diagnostic in the project/file adapter. */
export function nativeWritingOperationFailure(reason: unknown, packageKey: string): Error {
  return nativeFailure(reason, "writing_gateway_unavailable", packageKey, "projects");
}

/** Each user preview reads Core's switch and catalog anew. No retry, cached
 * business result, execution options or static fallback exists in enabled mode. */
export function createNativeWritingPreview(invokeCommand: typeof invoke = invoke): (sourcePath: string) => Promise<ManuscriptImportPreview> {
  return async sourcePath => {
    let discovered: unknown;
    try { discovered = await invokeCommand<unknown>("list_runtime_plugin_packages"); }
    catch (reason) { throw nativeFailure(reason, "plugin_discovery_failed"); }
    if (!enabled(discovered)) return invokeCommand<ManuscriptImportPreview>("preview_manuscript_import", {sourcePath});

    const resolution = resolveRuntimePluginDiscovery(discovered, coreCapabilities);
    const pkg = resolution.packages.find(candidate => candidate.manifest.id === PLUGIN_ID);
    if (!pkg) throw new NativeWritingPreviewError({code: "plugin_host_package_missing", pluginId: PLUGIN_ID,
      message: "原生导入预览试点已启用，但未发现写作插件包。"});
    const {manifest, packageKey} = pkg;
    if (manifest.native && manifest.native.protocolVersion !== WORKBENCH_NATIVE_PROTOCOL_VERSION)
      throw new PluginPackageError({code: "plugin_native_protocol_incompatible", pluginId: PLUGIN_ID,
        expectedVersion: WORKBENCH_NATIVE_PROTOCOL_VERSION, actualVersion: manifest.native.protocolVersion});
    if (manifest.formatVersion !== 2 || !manifest.native || hasPluginFrontend(manifest)
      || manifest.provides.length !== 1 || manifest.provides[0]?.id !== "writing.import-preview"
      || manifest.provides[0]?.version !== 1
      || manifest.dependencies.required.length || manifest.dependencies.optional.length
      || manifest.consumes.required.length || manifest.consumes.optional.length) {
      throw new NativeWritingPreviewError({code: "plugin_host_profile_unsupported", pluginId: PLUGIN_ID, packageKey,
        message: "原生试点包必须只提供 writing.import-preview v1，且无其他依赖或前端入口。"});
    }
    try { return await invokeCommand<ManuscriptImportPreview>("preview_runtime_manuscript_import", {packageKey, sourcePath}); }
    catch (reason) { throw nativeFailure(reason, "plugin_host_failed", packageKey); }
  };
}

export const previewWritingImport = createNativeWritingPreview();
