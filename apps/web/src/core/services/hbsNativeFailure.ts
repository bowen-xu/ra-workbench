import {nativeDataDiagnostic, nativeDiagnosticIdentity, nativeDiagnosticPackageKey} from "./nativeDataDiagnostic";
import {nativeTerminationDiagnostic, nativeTerminationMessage} from "./nativeTerminationDiagnostic";

const kinds = ["pureRejected", "sourceData", "technical"] as const;
const phases = ["sources", "source-read", "validation", "begin", "read", "write", "receipt", "commit", "postcommit-read", "rollback", "execution"] as const;
const outcomes = ["notStarted", "rolledBack", "committed", "unknown"] as const;
type SafeValue = string | number | Record<string, string | number>;
interface NativeFailure {
  readonly kind: typeof kinds[number];
  readonly phase: typeof phases[number];
  readonly outcome: typeof outcomes[number];
  readonly pluginId: "literature_library";
  readonly code: string;
  readonly [key: string]: SafeValue;
}
interface DecodedFailure {readonly diagnostic?: NativeFailure; readonly complete: boolean}
function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : undefined;
}
function member<T extends string>(values: readonly T[], value: unknown): value is T {
  return typeof value === "string" && values.some(entry => entry === value);
}
const identityFields = ["packageKey", "instanceId", "nativeRequestId", "operationId"] as const;
const dataFields = ["dataError", "databaseError", "rollbackError"] as const;
const scalarFields = ["exitCode", "signal"] as const;
const knownFields = new Set<string>(["kind", "phase", "outcome", "pluginId", "code", ...identityFields, ...dataFields, ...scalarFields, "terminationError"]);

function decode(value: unknown): DecodedFailure {
  const raw = record(value), code = nativeDiagnosticIdentity(raw?.code);
  if (!raw || !member(kinds, raw.kind) || !member(phases, raw.phase) || !member(outcomes, raw.outcome)
    || raw.pluginId !== "literature_library" || code === undefined) return {complete: false};
  const safe: NativeFailure = {kind: raw.kind, phase: raw.phase, outcome: raw.outcome, pluginId: "literature_library", code};
  let complete = Object.keys(raw).every(key => knownFields.has(key));
  for (const field of identityFields) {
    if (!Object.hasOwn(raw, field)) continue;
    const id = field === "packageKey" ? nativeDiagnosticPackageKey(raw[field]) : nativeDiagnosticIdentity(raw[field]);
    if (id === undefined) complete = false;
    else Object.assign(safe, {[field]: id});
  }
  for (const field of scalarFields) {
    if (!Object.hasOwn(raw, field)) continue;
    const scalar = raw[field];
    if (!Number.isSafeInteger(scalar) || Number(scalar) < -0x8000_0000 || Number(scalar) > 0x7fff_ffff) complete = false;
    else Object.assign(safe, {[field]: Number(scalar)});
  }
  for (const field of dataFields) {
    if (!Object.hasOwn(raw, field)) continue;
    const data = nativeDataDiagnostic(raw[field]);
    if (data === undefined) {complete = false; continue;}
    // Database failures share the Data vocabulary, without resource identity.
    const projected = field === "dataError" ? data : Object.fromEntries(Object.entries(data)
      .filter(([key]) => ["code", "phase", "safeCause", "sqliteCode", "sqliteExtendedCode"].includes(key)));
    Object.assign(safe, {[field]: projected});
  }
  if (Object.hasOwn(raw, "terminationError")) {
    const termination = nativeTerminationDiagnostic(raw.terminationError);
    if (termination === undefined) complete = false;
    else Object.assign(safe, {terminationError: termination});
  }
  return {diagnostic: safe, complete};
}

/** Preserve the old static envelope byte for byte. Only the finite new Native
 * marker adds diagnostics; arbitrary IPC fields never become an Error cause. */
export function hbsDocumentError(reason: unknown): unknown {
  const value = record(reason);
  if (!value || typeof value.message !== "string") return reason;
  const marked = Object.hasOwn(value, "nativeFailure");
  if (!marked && reason instanceof Error) return reason;
  const original = Object.assign(new Error(value.message),
    ...(typeof value.code === "string" ? [{code: value.code}] : []),
    ...(typeof value.requestId === "string" ? [{requestId: value.requestId}] : []));
  if (!marked) return original;
  const decoded = decode(value.nativeFailure), safe = decoded.diagnostic;
  const fields: string[] = [];
  for (const key of ["code", "requestId"] as const) {
    const id = nativeDiagnosticIdentity(value[key]);
    if (id !== undefined) fields.push(`${key}=${id}`);
    else delete (original as unknown as Record<string, unknown>)[key];
  }
  if (safe) for (const [key, entry] of Object.entries(safe)) {
    if (key === "terminationError") {
      const message = nativeTerminationMessage(entry);
      if (message !== undefined) fields.push(message);
    }
    else if (typeof entry === "object") fields.push(...Object.entries(entry).map(([name, part]) => `nativeFailure.${key}.${name}=${part}`));
    else fields.push(`nativeFailure.${key}=${entry}`);
  }
  // The own marker survives even when damaged, preventing a fallback to the
  // legacy MODEL-code cleanup rule. The private validity bit is not exported.
  const complete = decoded.complete && (!Object.hasOwn(value, completeMarker) || Reflect.get(value, completeMarker) === true);
  return Object.assign(new Error([value.message, ...fields].join(" · ")), original,
    {nativeFailure: safe, [completeMarker]: complete});
}
const completeMarker = Symbol("hbs-native-failure-complete");

/** Undefined keeps the original static cleanup policy. Native failures only
 * permit it after a matched, explicitly settled pure business rejection. */
export function hbsNativeRejectionIsDefinite(reason: unknown, requestId: string): boolean | undefined {
  const value = record(reason);
  if (!value || !Object.hasOwn(value, "nativeFailure")) return undefined;
  const decoded = decode(value.nativeFailure), safe = decoded.diagnostic;
  return decoded.complete && (!(reason instanceof Error) || Reflect.get(reason, completeMarker) === true)
    && safe?.kind === "pureRejected" && safe.code === "invalid_input"
    && ["notStarted", "rolledBack"].includes(safe.outcome) && value.requestId === requestId
    && typeof safe.packageKey === "string" && typeof safe.instanceId === "string" && typeof safe.nativeRequestId === "string"
    && ![...dataFields, ...scalarFields, "terminationError", "operationId"].some(key => Object.hasOwn(safe, key));
}

/** Appends only this HBS observer's safe Native fields; the generic diagnostic
 * exporter and every unrelated consumer retain their original contract. */
export function hbsNativeDiagnostic(reason: unknown): Record<string, unknown> {
  const safe = decode(record(reason)?.nativeFailure).diagnostic;
  return safe === undefined ? {} : {nativeFailure: safe};
}
