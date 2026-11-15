/** Shared finite Native/Data diagnostic projection. It never retains arbitrary
 * exception messages, SQL, document text or filesystem paths. */
function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === "object" && !Array.isArray(value) ? value as Record<string, unknown> : undefined;
}
export function nativeDiagnosticIdentity(value: unknown): string | undefined {
  return typeof value === "string" && /^[\w:.-]{1,200}$/u.test(value) ? value : undefined;
}
export function nativeDiagnosticPackageKey(value: unknown): string | undefined {
  return typeof value === "string" && /^[a-f0-9]{64}$/u.test(value) ? value : undefined;
}
function safeDataCause(value: unknown): string | undefined {
  const token = nativeDiagnosticIdentity(value);
  if (token !== undefined) return token;
  if (typeof value !== "string") return undefined;
  // SDK validation prints only fixed shape/type words and integer indices.
  const scalar = "(?:real data values must be finite|null is not allowed for this field|data value does not have the declared (?:Integer|Real|Text|Blob) type)";
  const shape = new RegExp(`^(?:${scalar}|data call id or version does not match its declaration|data call requires \\d{1,20} arguments, received \\d{1,20}|invalid data argument at index \\d{1,20}: ${scalar}|data row \\d{1,20} requires \\d{1,20} columns, received \\d{1,20}|invalid data value at row \\d{1,20}, column \\d{1,20}: ${scalar}|data result does not match the declared resource access)$`, "u");
  return shape.test(value) ? value : undefined;
}
export function nativeDataDiagnostic(value: unknown): Record<string, string | number> | undefined {
  const source = record(value);
  if (!source) return undefined;
  const safe: Record<string, string | number> = {};
  for (const key of ["code", "phase", "resourceId", "sqliteCode"] as const) {
    const token = nativeDiagnosticIdentity(source[key]);
    if (token !== undefined) safe[key] = token;
  }
  const cause = safeDataCause(source.safeCause);
  if (cause !== undefined) safe.safeCause = cause;
  const key = nativeDiagnosticPackageKey(source.packageKey);
  if (key !== undefined) safe.packageKey = key;
  if (Number.isSafeInteger(source.resourceVersion) && Number(source.resourceVersion) > 0 && Number(source.resourceVersion) <= 0xffff_ffff)
    safe.resourceVersion = Number(source.resourceVersion);
  if (Number.isSafeInteger(source.sqliteExtendedCode) && Number(source.sqliteExtendedCode) >= -0x8000_0000 && Number(source.sqliteExtendedCode) <= 0x7fff_ffff)
    safe.sqliteExtendedCode = Number(source.sqliteExtendedCode);
  return Object.keys(safe).length === 0 ? undefined : safe;
}
