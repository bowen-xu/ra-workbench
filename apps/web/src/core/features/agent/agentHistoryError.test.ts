import { expect, it } from "vitest";
import { agentHistoryErrorMessage } from "./agentHistoryError";

it("keeps snapshot and persistence diagnostics with the conversation identity", () => {
  expect(agentHistoryErrorMessage("historySnapshotExpired", "conversation-1")).toContain("historySnapshotExpired; conversation=conversation-1");
  expect(agentHistoryErrorMessage("historySnapshotTooLarge", "conversation-1")).toContain("已保留数据");
  const message = agentHistoryErrorMessage({ code: "data_persistence_failed", message: "projection blocked at position 12" }, "conversation-1");
  expect(message).toContain("projection blocked at position 12");
  expect(message).toContain("data_persistence_failed");
});

it("does not display unknown response objects", () => {
  const message = agentHistoryErrorMessage({ message: "private provider response", body: "opaque" }, "conversation-1");
  expect(message).toContain("agent_history_read_failed");
  expect(message).not.toContain("private");
  expect(message).not.toContain("opaque");
});

it("reports an incomplete legacy import without rendering arbitrary IPC text", () => {
  const message = agentHistoryErrorMessage({ code: "agent_legacy_migration_incomplete", message: "private body" }, "conversation-1");
  expect(message).toContain("旧阅读对话迁移尚未完成");
  expect(message).toContain("原历史已保留");
  expect(message).toContain("agent_legacy_migration_incomplete; conversation=conversation-1");
  expect(message).not.toContain("private body");
});
