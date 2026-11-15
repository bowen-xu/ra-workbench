import { diagnosticError } from "../../services/diagnosticError";
const nativeCodes = new Set(["invalidInput", "notFound", "conflict", "busy", "unavailable", "corrupt", "closed"]);
/** Preview errors retain safe identifiers without rendering arbitrary bodies. */
export function agentCanvasPreviewError(cause: unknown): string {
  const safe = diagnosticError(cause), details: string[] = [];
  const error = cause && typeof cause === "object" ? cause as Record<string, unknown> : {};
  const text = typeof cause === "string" ? cause : cause instanceof Error ? cause.message : "";
  const code = typeof safe.code === "string" ? safe.code
    : text.match(/^(?:AGENT_PREVIEW_[A-Z_]+|CANVAS_[A-Z_]+|agent_preview_[a-z_]+)(?=[:\s]|$)/u)?.[0]
      ?? (nativeCodes.has(text) ? text : undefined);
  if (code) details.push(code);
  if (typeof safe.httpStatus === "number" && Number.isInteger(safe.httpStatus) && safe.httpStatus >= 100 && safe.httpStatus <= 599) details.push(`HTTP ${safe.httpStatus}`);
  // Keep available operation identities, never arbitrary messages or payloads.
  for (const field of ["requestId", "operationId", "conversationId", "turnId", "fixtureId", "generation"] as const) {
    const value = error[field];
    if (typeof value === "string" && /^[\da-f]{8}-(?:[\da-f]{4}-){3}[\da-f]{12}$/iu.test(value)) details.push(`${field}=${value}`);
  }
  const message = code === "AGENT_PREVIEW_FIXTURE_CAPACITY"
    ? "预览场景已达上限，无法新建场景；当前对话和正在运行的任务保留。"
    : "无法确认预览操作结果，请保留当前场景并重试。";
  return `${message}${details.length ? `（${details.join("；")}）` : ""}`;
}
