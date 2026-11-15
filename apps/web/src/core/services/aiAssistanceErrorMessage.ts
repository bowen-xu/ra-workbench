import {paperContextErrorMessage} from "./paperContextErrorMessage";
import {nativeTerminationMessage} from "./nativeTerminationDiagnostic";

const sqlMessages = {
  DATA_SQL_INVALID: "数据写入参数无效。",
  DATA_SQL_UNMANAGED: "数据写入缺少受管存储所有者。",
  DATA_SQL_OPERATION_CONFLICT: "操作身份已对应其他内容，请核对原操作。",
  DATA_SQL_COMMIT_UNKNOWN: "数据提交结果尚未确认，请重开后核对原操作。",
  DATA_OPERATION_PARTIAL: "操作部分完成，请保留原请求并核对。",
  DATA_OPERATION_OUTCOME_UNKNOWN: "外部操作结果尚未确认，请保留原请求并核对。",
  DATA_SQL_RECOVERY_REQUIRED: "数据存储需要重开核对，当前不能继续写入。",
  DATA_SQL_UNAVAILABLE: "数据存储暂不可用，请保留原操作身份并重试读取。",
  DATA_SQL_CORRUPT: "数据存储校验失败，请保留数据并核对恢复状态。",
} as const;

function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown> : undefined;
}

function technicalToken(value: unknown): value is string {
  return typeof value === "string" && /^[a-zA-Z][a-zA-Z0-9_-]{0,100}$/u.test(value);
}

function uuid(value: unknown): value is string {
  return typeof value === "string" && /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/iu.test(value);
}

function position(value: unknown): value is string {
  return typeof value === "string" && /^(?:0|[1-9][0-9]{0,19})$/u.test(value)
    && (value.length < 20 || value <= "18446744073709551615");
}

/** Finite Core preparation diagnostics, never a raw child/provider payload. */
function nativePreparationMessage(
  reason: unknown,
  context?: {readonly conversationId: string; readonly requestId: string},
): string | undefined {
  const outer = record(reason);
  const value = outer?.code === "preparation" ? record(outer.reason) : outer;
  const writing = value?.pluginId === "writing_projects";
  const independent = writing && context !== undefined;
  if (!value || !(writing || value.pluginId === "literature_library")
    || typeof value.code !== "string" || !(writing ? /^(?:(?:plugin_|runtime_plugin_)[a-z_]+|storage_error|writing_(?:file_kind_policy|context_catalog|scope)_[a-z_]+)$/u : /^(?:plugin_host_|plugin_data_|runtime_plugin_)[a-z_]+$/u).test(value.code)
    || typeof value.message !== "string"
    || !(independent ? uuid(context?.conversationId) && uuid(context?.requestId)
      : uuid(value.conversationId) && (writing ? uuid(value.manuscriptId) && uuid(value.operationId) : uuid(value.requestId) && uuid(value.pageId)))) return undefined;
  const fields = [`错误码：${value.code}`, `conversationId=${independent ? context?.conversationId : value.conversationId}`,
    ...(independent ? [`requestId=${context?.requestId}`] : writing ? [`manuscriptId=${value.manuscriptId}`, `operationId=${value.operationId}`]
      : [`requestId=${value.requestId}`, `pageId=${value.pageId}`]), `pluginId=${value.pluginId}`];
  for (const key of writing ? ["instanceId", "requestId"] as const : ["instanceId", "nativeRequestId"] as const) {
    if (uuid(value[key])) fields.push(`${writing && key === "requestId" ? "nativeRequestId" : key}=${value[key]}`);
  }
  if (typeof value.packageKey === "string" && /^[0-9a-f]{64}$/u.test(value.packageKey)) fields.push(`packageKey=${value.packageKey}`);
  if (typeof value.resourceId === "string" && /^[a-z][a-z0-9_.-]{0,159}$/u.test(value.resourceId)) fields.push(`resourceId=${value.resourceId}`);
  for (const key of ["phase", "safeCause", "sqliteCode"] as const) {
    if (technicalToken(value[key])) fields.push(`${key}=${value[key]}`);
  }
  if (typeof value.resourceVersion === "number" && Number.isInteger(value.resourceVersion)
    && value.resourceVersion > 0 && value.resourceVersion <= 4_294_967_295) fields.push(`resourceVersion=${value.resourceVersion}`);
  for (const key of ["exitCode", "signal", "sqliteExtendedCode"] as const) {
    const number = value[key];
    if (typeof number === "number" && Number.isInteger(number)
      && number >= -2_147_483_648 && number <= 2_147_483_647) fields.push(`${key}=${number}`);
  }
  // Core only emits safe cleanup categories. Unknown properties, raw stderr,
  // SQL, arguments, input paths and document contents are never formatted.
  if (typeof value.cleanupError === "string" && /^[\p{L}\p{N}_:：().,，；（） -]{1,300}$/u.test(value.cleanupError.replaceAll("I/O", "IO").replaceAll("／", ""))) {
    fields.push(`cleanup=${value.cleanupError}`);
  }
  const termination = nativeTerminationMessage(value.terminationError);
  if (termination !== undefined) fields.push(termination);
  if (writing && record(value.dataError)) {
    fields.push(...writingSourceFields({kind: "data", error: value.dataError}));
  }
  return `${value.message}（${fields.join("；")}）`;
}

/** Decode only the native SQL diagnostic DTO, never arbitrary provider JSON. */
function sqlMessage(reason: unknown): string | undefined {
  const outer = record(reason);
  const failure = outer?.code === "storage" ? record(outer.reason) : outer;
  if (!failure || typeof failure.code !== "string"
    || !Object.hasOwn(sqlMessages, failure.code)) return undefined;
  const code = failure.code as keyof typeof sqlMessages;
  const diagnostic = record(failure.reason);
  const fields = [`错误码：${code}`];
  for (const key of ["conversationId", "manuscriptId", "requestId"] as const) {
    if (uuid(failure[key])) fields.push(`${key}=${failure[key]}`);
  }
  if (diagnostic) {
    if (uuid(diagnostic.operationId)) fields.push(`operationId=${diagnostic.operationId}`);
    if (uuid(diagnostic.epoch)) fields.push(`epoch=${diagnostic.epoch}`);
    // Keep decimal positions as text. Converting to Number can change the
    // original receipt identity above JavaScript's safe integer range.
    if (position(diagnostic.sqlFence)) fields.push(`sqlFence=${diagnostic.sqlFence}`);
    if (technicalToken(diagnostic.phase)) fields.push(`phase=${diagnostic.phase}`);
    if (technicalToken(diagnostic.safeCause)) fields.push(`cause=${diagnostic.safeCause}`);
    if (technicalToken(diagnostic.sqliteCode)) fields.push(`SQLite=${diagnostic.sqliteCode}`);
    if (typeof diagnostic.sqliteExtendedCode === "number"
      && Number.isInteger(diagnostic.sqliteExtendedCode)
      && diagnostic.sqliteExtendedCode >= 0 && diagnostic.sqliteExtendedCode <= 2_147_483_647) {
      fields.push(`sqliteExtendedCode=${diagnostic.sqliteExtendedCode}`);
    }
  }
  fields.push(...writingSourceFields(failure.source));
  return `${sqlMessages[code]}（${fields.join("；")}）`;
}

/** Only the Core-owned writing diagnostic DTO can add source provenance. */
function writingSourceFields(value: unknown): string[] {
  const source = record(value);
  const fields: string[] = [];
  if (source?.kind === "native") {
    if (typeof source.packageKey === "string" && /^[0-9a-f]{64}$/u.test(source.packageKey)) fields.push(`packageKey=${source.packageKey}`);
    if (uuid(source.instanceId)) fields.push(`instanceId=${source.instanceId}`);
    if (uuid(source.requestId)) fields.push(`nativeRequestId=${source.requestId}`);
    if (typeof source.rejection === "string" && ["invalidInput", "conflict", "unavailable", "budget"].includes(source.rejection)) fields.push(`source.rejection=${source.rejection}`);
  }
  if (source?.kind === "data" || source?.kind === "database") {
    const error = record(source.error);
    for (const key of ["code", "phase", "safeCause", "sqliteCode"] as const) {
      const token = error?.[key];
      if (technicalToken(token)) fields.push(`source.${key}=${token}`);
    }
    if (typeof error?.resourceId === "string" && /^[a-z][a-z0-9_.-]{0,159}$/u.test(error.resourceId)) fields.push(`resourceId=${error.resourceId}`);
    if (typeof error?.packageKey === "string" && /^[0-9a-f]{64}$/u.test(error.packageKey)) fields.push(`packageKey=${error.packageKey}`);
    if (typeof error?.resourceVersion === "number" && Number.isInteger(error.resourceVersion)
      && error.resourceVersion > 0 && error.resourceVersion <= 4_294_967_295) fields.push(`resourceVersion=${error.resourceVersion}`);
    if (typeof error?.sqliteExtendedCode === "number" && Number.isInteger(error.sqliteExtendedCode)
      && error.sqliteExtendedCode >= -2_147_483_648 && error.sqliteExtendedCode <= 2_147_483_647) fields.push(`source.sqliteExtendedCode=${error.sqliteExtendedCode}`);
  }
  return fields;
}

function queuedWritingMessage(reason: unknown): string | undefined {
  const outer = record(reason);
  const scope = record(outer?.writingSource);
  if (!outer || !scope || typeof outer.code !== "string" || !["storage", "queue", "preparation"].includes(outer.code)
    || !uuid(scope.conversationId) || !uuid(scope.manuscriptId)
    || !uuid(scope.operationId) || !uuid(scope.inputId)) return undefined;
  const source = record(scope.source);
  if (!source || typeof source.kind !== "string" || !["native", "nativeFailure", "data", "database", "unavailable"].includes(source.kind)) return undefined;
  if (source.kind === "nativeFailure") {
    const error = record(source.error);
    const message = nativePreparationMessage({...error, conversationId: scope.conversationId,
      manuscriptId: scope.manuscriptId, operationId: scope.operationId});
    if (message !== undefined) return `${message}（inputId=${scope.inputId}）`;
  }
  const original = sqlMessage(outer) ?? "写作队列输入准备失败。";
  const fields = [`conversationId=${scope.conversationId}`, `manuscriptId=${scope.manuscriptId}`,
    `operationId=${scope.operationId}`, `inputId=${scope.inputId}`];
  if (technicalToken(outer.code)) fields.push(`错误码：${outer.code}`);
  if (outer.code === "preparation" && typeof outer.reason === "string"
    && ["unbound", "invalidInput", "conflict", "notFound", "contextUnavailable", "contextBudget", "modelUnavailable", "unavailable"].includes(outer.reason)) fields.push(`reason=${outer.reason}`);
  const domain = record(outer.reason);
  if (outer.code === "queue" && typeof domain?.code === "string"
    && ["invalid", "conflict", "busy", "paused", "capacity", "closed", "corrupt", "unavailable"].includes(domain.code)) fields.push(`reason=${domain.code}`);
  fields.push(...writingSourceFields(source));
  return `${original}（${fields.join("；")}）`;
}

/** Display-only provenance from the finite writing preparation error DTOs. */
export function writingPreparationSourceMessage(reason: unknown): string | undefined {
  const queued = queuedWritingMessage(reason);
  if (queued !== undefined) return queued;
  const outer = record(reason);
  if (outer?.pluginId === "writing_projects") return nativePreparationMessage(outer);
  const failure = outer?.code === "storage" ? record(outer.reason) : undefined;
  if (failure && record(failure.source) && uuid(failure.conversationId)
    && uuid(failure.manuscriptId) && uuid(failure.requestId)) return sqlMessage(outer);
  return undefined;
}

/** Ordinary context carries only its actual conversation and request identities. */
function contextPreparationSourceMessage(reason: unknown): string | undefined {
  const outer = record(reason);
  const scope = record(outer?.contextSource);
  const source = record(scope?.source);
  if (outer?.code !== "storage" || !uuid(scope?.conversationId) || !uuid(scope?.requestId)
    || !source || typeof source.kind !== "string"
    || !["native", "nativeFailure", "data", "database", "unavailable"].includes(source.kind)) return undefined;
  const original = sqlMessage(outer);
  if (original === undefined) return undefined;
  const fields = [
    `conversationId=${scope.conversationId}`, `requestId=${scope.requestId}`,
    ...writingSourceFields(source),
  ];
  if (source.kind === "nativeFailure") {
    const native = nativePreparationMessage(source.error, {conversationId: scope.conversationId, requestId: scope.requestId});
    if (native !== undefined) fields.push(native);
  }
  return `${original}（${fields.join("；")}）`;
}

export function aiAssistanceErrorMessage(reason: unknown): string {
  const paper = paperContextErrorMessage(reason);
  if (paper !== undefined) {
    const sql = sqlMessage(reason);
    return sql === undefined ? paper : `${sql} · ${paper}`;
  }
  const context = contextPreparationSourceMessage(reason);
  if (context !== undefined) return context;
  const writing = writingPreparationSourceMessage(reason);
  if (writing !== undefined) return writing;
  const sql = sqlMessage(reason);
  if (sql !== undefined) return sql;
  const native = nativePreparationMessage(reason);
  if (native !== undefined) return native;
  if (typeof reason === "object" && reason !== null && !Array.isArray(reason)) {
    const {message, code} = reason as { readonly message?: unknown; readonly code?: unknown };
    if (code === "AGENT_PREVIEW_OWNER_INTENT_MISSING") return "原请求的归属尚无法确认，原请求和草稿已保留。（错误码：AGENT_PREVIEW_OWNER_INTENT_MISSING）";
    if (typeof message === "string") return typeof code === "string" && /^[a-z][a-zA-Z0-9_:-]{0,100}$/u.test(code)
      ? `${message}（错误码：${code}）` : message;
  }
  return String(reason);
}
