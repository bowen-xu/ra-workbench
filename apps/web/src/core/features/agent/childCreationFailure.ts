/** Classify only failures tied to this exact saved child operation. */
export function childCreationFailureStage(reason: string | undefined,
  child: { readonly childId: string; readonly operationId: string; readonly submissionRequestId?: string }, conversationId: string, turnId: string): "creation" | "cleanup" | undefined {
  const prefix = "tool_outcome_unknown: ";
  if (!reason?.startsWith(prefix)) return undefined;
  try {
    const value: unknown = JSON.parse(reason.slice(prefix.length));
    if (!value || typeof value !== "object") return undefined;
    const detail = value as Record<string, unknown>;
    if (detail.resultId !== child.childId) return undefined;
    if (detail.code === "CHILD_CREATION_UNCONFIRMED" && detail.conversationId === conversationId
      && detail.turnId === turnId && detail.requestId === child.operationId) return "creation";
    if (!child.submissionRequestId || detail.requestId !== child.submissionRequestId
      || (detail.conversationId != null && detail.conversationId !== conversationId)
      || (detail.turnId != null && detail.turnId !== turnId)) return undefined;
    if (detail.code === "CHILD_CLEANUP_UNCONFIRMED" && detail.childArchiveIntentVersion == null
      && detail.childArchiveOperationId == null && detail.journalRequestId == null) return "cleanup";
    const codes = ["CHILD_CLEANUP_UNCONFIRMED", "DATA_SQL_INVALID", "DATA_SQL_UNMANAGED", "DATA_SQL_OPERATION_CONFLICT",
      "DATA_SQL_COMMIT_UNKNOWN", "DATA_OPERATION_PARTIAL", "DATA_OPERATION_OUTCOME_UNKNOWN", "DATA_SQL_RECOVERY_REQUIRED",
      "DATA_SQL_UNAVAILABLE", "DATA_SQL_CORRUPT"];
    // The backend validates the v5 operation against the saved intent during
    // inspection; presentation never grants recovery authority from this match.
    return typeof detail.code === "string" && codes.includes(detail.code) && detail.childArchiveIntentVersion === 1
      && typeof detail.childArchiveOperationId === "string" && /^[0-9a-f]{8}-[0-9a-f]{4}-5[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/iu.test(detail.childArchiveOperationId)
      && detail.journalRequestId === detail.childArchiveOperationId && Number.isSafeInteger(detail.childArchiveExpectedWatermark)
      && (detail.childArchiveExpectedWatermark as number) >= 0 ? "cleanup" : undefined;
  } catch {
    return undefined;
  }
}

export function isMatchingChildCreationFailure(reason: string | undefined,
  child: { readonly childId: string; readonly operationId: string; readonly submissionRequestId?: string }, conversationId: string, turnId: string): boolean {
  return childCreationFailureStage(reason, child, conversationId, turnId) !== undefined;
}
