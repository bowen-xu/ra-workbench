import {nativeDataDiagnostic as dataDiagnostic, nativeDiagnosticIdentity as identity, nativeDiagnosticPackageKey as packageKey} from "./nativeDataDiagnostic";
import {nativeTerminationDiagnostic, nativeTerminationMessage} from "./nativeTerminationDiagnostic";

function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : undefined;
}

/** Only the three explicit reading-source commands use this Native envelope.
 * Keep the original static error unchanged and never retain an arbitrary IPC
 * payload as cause: it may contain document text, SQL or filesystem paths. */
export function structuredSourceError(reason: unknown): Error | undefined {
  const value = record(reason);
  if (typeof value?.code !== "string" || typeof value.message !== "string") return undefined;
  const original = Object.assign(new Error(value.message), {code: value.code});
  if (value.pluginId !== "literature_library") return original;
  const safe: Record<string, unknown> = {pluginId: "literature_library"};
  const fields: string[] = [];
  const code = identity(value.code);
  if (code !== undefined) {safe.code = code; fields.push(`code=${code}`);}
  for (const name of ["packageKey", "instanceId", "requestId"] as const) {
    const id = name === "packageKey" ? packageKey(value[name]) : identity(value[name]);
    if (id !== undefined) {safe[name] = id; fields.push(`${name}=${id}`);}
  }
  const dataError = dataDiagnostic(value.dataError);
  if (dataError !== undefined) {
    safe.dataError = dataError;
    fields.push(...Object.entries(dataError).map(([key, entry]) => `dataError.${key}=${entry}`));
  }
  const termination = nativeTerminationDiagnostic(value.terminationError);
  if (termination !== undefined) {
    safe.terminationError = termination;
    fields.push(nativeTerminationMessage(termination)!);
  }
  return Object.assign(new Error([value.message, ...fields].join(" · ")), safe);
}
