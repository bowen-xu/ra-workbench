/** Diagnostics have no document bodies, titles, asset bytes or raw requests. */
export function diagnosticError(reason: unknown): Record<string, unknown> {
  const error = reason && typeof reason === "object" ? reason as Record<string, unknown> : {};
  const text = reason instanceof Error ? reason.message : typeof error.message === "string" ? error.message : String(reason);
  const codes = [...new Set(text.match(/\b(?:MODEL|MOCK_NOTE|NOTE_RECOVERY)_[A-Z_]+\b/g) ?? [])];
  const safe = (value: unknown) => typeof value === "string" && /^[\w:.-]{1,200}$/.test(value) ? value : undefined;
  const requestId = safe(error.requestId), turnId = safe(error.turnId);
  const operationId = safe(error.operationId), taskId = safe(error.taskId);
  return {name: safe(error.name), code: safe(error.code), codes,
    ...(typeof error.status === "number" ? {httpStatus:error.status} : {}),
    ...(requestId ? {requestId} : {}),
    ...(turnId ? {turnId} : {}),
    ...(operationId ? {operationId} : {}),
    ...(taskId ? {taskId} : {}),
    ...(typeof error.osError === "number" ? {osError:error.osError} : {}),
    frames:typeof error.stack==="string"?error.stack.split("\n").slice(1,17).flatMap(line=>{
      const match=line.match(/([^/\\\s:()]+\.[cm]?[jt]sx?):(\d+):(\d+)/);
      return match?[{file:match[1],line:Number(match[2]),column:Number(match[3])}]:[];
    }):[],
    // Unknown messages may contain private content. Preserve technical causes
    // by category rather than exporting an arbitrary exception payload.
    cause: /quota|配额|空间不足/i.test(text) ? "storage_quota" : /permission|权限|denied/i.test(text) ? "permission_denied"
      : /abort|取消/i.test(text) ? "aborted" : /corrupt|损坏|invalid|无效/i.test(text) ? "invalid_data"
      : /timeout|超时/i.test(text) ? "timeout" : /读取|read/i.test(text) ? "read_failed"
      : /写入|write/i.test(text) ? "write_failed" : "unclassified",
    ...(error.cause && error.cause !== reason ? {underlying: diagnosticErrorShallow(error.cause)} : {}),
  };
}
function diagnosticErrorShallow(reason: unknown): Record<string, unknown> {
  const error = reason && typeof reason === "object" ? reason as Record<string, unknown> : {};
  return {name: typeof error.name === "string" && /^[\w.-]{1,100}$/.test(error.name) ? error.name : undefined,
    code: typeof error.code === "string" && /^[\w.-]{1,100}$/.test(error.code) ? error.code : undefined};
}
