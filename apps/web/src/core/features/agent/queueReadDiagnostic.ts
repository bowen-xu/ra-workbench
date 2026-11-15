import { decodeAgentSqlImmediateFailure } from "@research-assistant/core/agent";
import { aiAssistanceErrorMessage } from "../../services/aiAssistanceErrorMessage";
import { diagnosticError } from "../../services/diagnosticError";

const token = /^(?:invalid|conflict|busy|paused|capacity|closed|corrupt|unavailable|(?:AGENT|DATA|QUEUE)_[A-Z0-9_]+|agent_preview_[a-z0-9_]+)$/u;
const uuid = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/iu;

/** Show only safe diagnostics, never queue contents or arbitrary exception text. */
export function queueReadDiagnostic(failure: {readonly cause: unknown; readonly scope: unknown}): string {
  const {cause, scope} = failure;
  const safe = diagnosticError(cause);
  const object = cause && typeof cause === "object" ? cause as Record<string, unknown> : {};
  const message = cause instanceof Error ? cause.message : typeof cause === "string" ? cause : "";
  const messageCode = message.match(/\b(?:AGENT_[A-Z0-9_]+|agent_preview_[a-z0-9_]+|DATA_[A-Z0-9_]+|QUEUE_[A-Z0-9_]+)\b/u)?.[0];
  const rawCode = typeof object.code === "string" ? object.code : messageCode ?? message;
  const code = token.test(rawCode) ? rawCode : cause instanceof Error && cause.name === "AgentSubmissionDecodeError"
    ? "QUEUE_RESPONSE_INVALID" : "QUEUE_READ_FAILED";
  const details = [`code=${code}`];
  if (scope && typeof scope === "object") {
    const value = scope as Record<string, unknown>;
    if (typeof value.conversationId === "string" && uuid.test(value.conversationId)) details.push(`C=${value.conversationId}`);
    if (typeof value.manuscriptId === "string" && uuid.test(value.manuscriptId)) details.push(`M=${value.manuscriptId}`);
    if (typeof value.paperId === "string" && uuid.test(value.paperId)) details.push(`paperId=${value.paperId}`);
    if (typeof value.resourceId === "string" && uuid.test(value.resourceId)) details.push(`resourceId=${value.resourceId}`);
    if (typeof value.kind === "string" && ["manuscript", "paper", "note", "readingNote", "context"].includes(value.kind)) details.push(`scope=${value.kind}`);
  }
  if (typeof safe.httpStatus === "number" && Number.isInteger(safe.httpStatus) && safe.httpStatus >= 100 && safe.httpStatus <= 599)
    details.push(`HTTP=${safe.httpStatus}`);
  if (typeof safe.requestId === "string" && uuid.test(safe.requestId)) details.push(`requestId=${safe.requestId}`);
  if (typeof safe.name === "string") details.push(`error=${safe.name}`);
  if (typeof safe.cause === "string") details.push(`cause=${safe.cause}`);
  if (/^(?:failed to fetch|networkerror when attempting to fetch resource\.?|network request failed)$/iu.test(message.trim()))
    details.push("transport=network_error");
  if (typeof safe.osError === "number" && Number.isInteger(safe.osError)) details.push(`osError=${safe.osError}`);
  if (safe.underlying && typeof safe.underlying === "object") {
    const underlying = safe.underlying as Record<string, unknown>;
    if (typeof underlying.name === "string") details.push(`underlying=${underlying.name}`);
    if (typeof underlying.code === "string" && token.test(underlying.code)) details.push(`underlyingCode=${underlying.code}`);
  }
  if (Array.isArray(safe.frames)) {
    const frames = safe.frames.flatMap((value: unknown) => {
      if (!value || typeof value !== "object") return [];
      const frame = value as Record<string, unknown>;
      return typeof frame.file === "string" && /^[\w.-]+$/u.test(frame.file) && typeof frame.line === "number"
        && Number.isSafeInteger(frame.line) && typeof frame.column === "number" && Number.isSafeInteger(frame.column)
        ? [`${frame.file}:${frame.line}:${frame.column}`] : [];
    });
    if (frames.length) details.push(`stack=${frames.join(" <- ")}`);
  }
  const sql = decodeAgentSqlImmediateFailure(cause);
  return `读取队列失败，请重试。（${details.join("；")}）${sql ? aiAssistanceErrorMessage(cause) : ""}`;
}
