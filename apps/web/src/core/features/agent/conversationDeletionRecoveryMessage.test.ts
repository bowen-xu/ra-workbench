import { expect, it } from "vitest";
import type { ConversationDeletionSnapshot } from "./conversationDeletionController";
import { deletionRecoveryAction, deletionRecoveryMessage } from "./conversationDeletionRecoveryMessage";
import { assertConversationOperationStorageCurrent } from "./conversationOperationLock";
import { registerStoragePersistence } from "../../runtime/storagePersistence";
import { createMockAgentRecoveryStorage } from "../../../app/runtime/mock/MockAgentRecoveryStorage";
const id = "00000000-0000-4000-8000-000000000001", operation = "00000000-0000-4000-8000-000000000002";
const state: ConversationDeletionSnapshot = {phase: "unknown", record: {schemaVersion: 1, intent: {schemaVersion: 1,
  requestId: operation, conversationId: id, binding: null, expectedTitle: "Synthetic", expectedDurableCount: 0, force: true},
  internal: false, epoch: id, stage: "unknown", reason: null, receipt: null}, status: null, cause: "unavailable", localIssue: null};
it("keeps the original operation and known HTTP/code without exposing arbitrary provider content", () => {
  const message = deletionRecoveryMessage({...state, cause: {code: "agent_preview_operation_unconfirmed", status: 503,
    message: "private provider body", body: "private request", authorization: "private header"}});
  expect(message).toContain(operation); expect(message).toContain("agent_preview_operation_unconfirmed"); expect(message).toContain("HTTP 503");
  expect(message).not.toContain("private");
  expect(deletionRecoveryMessage(state)).toContain("code=unavailable");
});
it("separates read, exact retry and local completion and does not infer force authorization from a missing proof", () => {
  expect(deletionRecoveryAction(state)).toBe("核对原删除");
  expect(deletionRecoveryAction({...state, phase: "retryable"})).toBe("重试原删除");
  expect(deletionRecoveryAction({...state, phase: "committed"})).toBe("完成本机收尾");
  expect(deletionRecoveryAction({...state, localIssue: Error("DELETION_RECOVERY_UNAVAILABLE")})).toBe("重试本机恢复记录");
  expect(deletionRecoveryMessage({...state, phase: "retryable"})).toContain("文件清理可能已发生");
});
it("keeps receipt mismatch and data environment errors actionable using only original identity", () => {
  for (const code of ["DELETION_RECOVERY_EPOCH_CHANGED", "DELETION_RECOVERY_PROOF_MISSING", "DELETION_STATUS_MISMATCH", "DELETION_RECEIPT_MISMATCH"]) {
    const message = deletionRecoveryMessage({...state, cause: Error(code)});
    expect(message).toContain(code); expect(message).toContain(operation);
  }
});
it("maps the existing preview stale-storage fact to the owning catalog operation", async () => {
  const storage = createMockAgentRecoveryStorage("synthetic");
  registerStoragePersistence(storage, {backend: "IndexedDB", pending: false, error: undefined, resetting: false,
    flush: async () => undefined, retry: async () => undefined, reset: async () => undefined,
    subscribe: () => () => undefined, close: () => undefined,
    assertCurrent: async () => {throw Error("FORK_RECOVERY_STORAGE_STALE");}});
  await expect(assertConversationOperationStorageCurrent(storage, "DELETION_RECOVERY")).rejects.toThrow("DELETION_RECOVERY_STORAGE_STALE");
  await expect(assertConversationOperationStorageCurrent(storage, "FORK_RECOVERY")).rejects.toThrow("FORK_RECOVERY_STORAGE_STALE");
});
