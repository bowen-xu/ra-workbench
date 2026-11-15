import {hbsDocumentError} from "./hbsNativeFailure";
import {nativeDataDiagnostic, nativeDiagnosticIdentity, nativeDiagnosticPackageKey} from "./nativeDataDiagnostic";
import {nativeTerminationMessage} from "./nativeTerminationDiagnostic";

/** Display the finite Core-owned paper, retrieval, history and index errors. */
export function paperContextErrorMessage(reason: unknown): string | undefined {
  if (reason === null || typeof reason !== "object" || Array.isArray(reason)) return undefined;
  const value = reason as Record<string, unknown>;
  const retrieval = value.retrievalOperation === "preview" || value.retrievalOperation === "trace-read";
  const history = value.analysisHistoryOperation === "get" || value.analysisHistoryOperation === "latest";
  const index = typeof value.indexOperation === "string"
    && ["status", "job", "retry", "rebuild", "reconcile", "provider-backfill", "execution"].includes(value.indexOperation);
  if ((!retrieval && !index && !history && value.paperContextOperation !== "prepare" && value.paperContextOperation !== "preview" && value.paperContextOperation !== "submission")
    || typeof value.message !== "string" || typeof value.code !== "string") return undefined;
  const fields: string[] = [];
  const code = nativeDiagnosticIdentity(value.code);
  if (code !== undefined) fields.push(`code=${code}`);
  let message = value.message;
  if (Object.hasOwn(value, "nativeFailure")) {
    const projected = hbsDocumentError(value);
    if (projected instanceof Error) message = projected.message;
  } else {
    const key = nativeDiagnosticPackageKey(value.packageKey);
    if (key !== undefined) fields.push(`packageKey=${key}`);
    for (const field of ["instanceId", "requestId"] as const) {
      const id = nativeDiagnosticIdentity(value[field]);
      if (id !== undefined) fields.push(`${field === "requestId" ? "nativeRequestId" : field}=${id}`);
    }
    if (index) {
      const id = nativeDiagnosticIdentity(value.nativeRequestId);
      if (id !== undefined) fields.push(`nativeRequestId=${id}`);
    }
    const data = nativeDataDiagnostic(value.dataError);
    if (data !== undefined) {
      for (const [key, part] of Object.entries(data)) fields.push(`dataError.${key}=${part}`);
    }
    for (const field of ["exitCode", "signal"] as const) {
      const scalar = value[field];
      if (Number.isSafeInteger(scalar) && Number(scalar) >= -0x8000_0000 && Number(scalar) <= 0x7fff_ffff) {
        fields.push(`${field}=${scalar}`);
      }
    }
    const termination = nativeTerminationMessage(value.terminationError);
    if (termination !== undefined) fields.push(termination);
  }
  const scopeFields = index
    ? ["jobId", "representationId", "embeddingSpaceId", "operationId"] as const
    : history
      ? ["workflowId", "paperId"] as const
      : retrieval
      ? ["clientRequestId", "traceId"] as const
      : value.paperContextOperation === "submission"
        ? ["conversationId", "requestId", "paperId", "paperContextSnapshotId"] as const
        : ["conversationId", "turnId", "paperId"] as const;
  for (const field of scopeFields) {
    const id = nativeDiagnosticIdentity(value[field]);
    if (id !== undefined) fields.push(`${field}=${id}`);
  }
  if (index) {
    const cause = nativeDataDiagnostic(value.indexCause);
    if (cause !== undefined) {
      for (const [key, part] of Object.entries(cause)) fields.push(`indexCause.${key}=${part}`);
    }
    if (value.outcome === "unknown") fields.push("outcome=unknown");
  }
  if (retrieval) {
    for (const field of ["leaseReleaseConfirmed", "traceWriteStarted"] as const) {
      if (typeof value[field] === "boolean") fields.push(`${field}=${value[field]}`);
    }
    for (const field of ["providerError", "semanticError"] as const) {
      const cause = value[field];
      if (cause === null || typeof cause !== "object" || Array.isArray(cause)) continue;
      const detail = cause as Record<string, unknown>;
      const code = nativeDiagnosticIdentity(detail.code);
      if (code !== undefined) fields.push(`${field}.code=${code}`);
      if (field === "providerError" && Number.isSafeInteger(detail.retryAfterMs) && Number(detail.retryAfterMs) >= 0) {
        fields.push(`providerError.retryAfterMs=${detail.retryAfterMs}`);
      }
    }
    if (value.outcome === "unknown") fields.push("outcome=unknown");
  }
  if (value.imageObjectWriteStarted === true) fields.push("imageObjectWriteStarted=true");
  if (Number.isSafeInteger(value.savedImageAssetCount) && Number(value.savedImageAssetCount) >= 0) {
    fields.push(`savedImageAssetCount=${value.savedImageAssetCount}`);
  }
  return [message, ...fields].join(" · ");
}
