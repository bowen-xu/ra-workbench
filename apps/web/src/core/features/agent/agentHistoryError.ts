const known = new Set(["invalidInput", "notFound", "busy", "recoveryRequired", "corrupt", "unavailable"]);

/** Native persistence diagnostics are already safe at the owner boundary. Do
 * not serialize arbitrary IPC objects or provider bodies into notifications. */
export function agentHistoryErrorMessage(error: unknown, conversationId: string): string {
  let code = "agent_history_read_failed";
  let message = "无法读取对话历史，请重新打开对话。";
  if (error === "historySnapshotExpired") {
    code = "historySnapshotExpired";
    message = "历史快照已过期，请重新打开对话读取。";
  } else if (error === "historySnapshotTooLarge") {
    code = "historySnapshotTooLarge";
    message = "对话历史超过当前快照容量，已保留数据。";
  } else if (typeof error === "string" && known.has(error)) code = error;
  else if (error && typeof error === "object" && "code" in error && error.code === "agent_legacy_migration_incomplete") {
    code = error.code;
    message = "旧阅读对话迁移尚未完成，原历史已保留；请核对迁移状态。";
  }
  else if (error && typeof error === "object" && "code" in error && error.code === "data_persistence_failed") {
    code = error.code;
    if ("message" in error && typeof error.message === "string") message = error.message;
  }
  return `${message} [${code}; conversation=${conversationId}]`;
}
