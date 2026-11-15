import { expect, it } from "vitest";
import { conversationDeletionDiagnostic } from "./conversationDeletionDiagnostic";
const requestId = "11111111-1111-4111-8111-111111111111";
it("retains matching safe storage evidence and never includes private SQL/body fields", () => {
  const value = {code:"storage",status:503,reason:{code:"DATA_SQL_COMMIT_UNKNOWN",reason:{operationId:requestId,
    epoch:"22222222-2222-4222-8222-222222222222",sqlFence:"18446744073709551615",phase:"commit",safeCause:"sql_commit_outcome_unknown",
    sqliteCode:"ConstraintViolation",sqliteExtendedCode:787,message:"private trigger body",sql:"private SQL",requestBody:"private input"}}};
  const message = conversationDeletionDiagnostic(value,requestId,{force:true})!;
  for(const field of ["DATA_SQL_COMMIT_UNKNOWN",requestId,"18446744073709551615","commit","787","HTTP 503"]) expect(message).toContain(field);
  expect(message).not.toContain("private"); expect(message).toContain("文件清理可能已经执行");
  expect(conversationDeletionDiagnostic(value,requestId)).not.toContain("文件清理可能已经执行");
});
it("does not adopt foreign operation evidence or arbitrary backend bodies", () => {
  expect(conversationDeletionDiagnostic({code:"DATA_SQL_COMMIT_UNKNOWN",reason:{operationId:"22222222-2222-4222-8222-222222222222"}},requestId)).toBeNull();
  for(const cause of [new Error("private provider reply"),{message:"private body"},"unavailable"])expect(conversationDeletionDiagnostic(cause,requestId)).toBeNull();
});
it("blocks immutable identity/capability failures but keeps original unknown-result retries", async () => {
  const {conversationDeletionStorageBlocked} = await import("./conversationDeletionDiagnostic");
  for(const code of ["DATA_SQL_INVALID","DATA_SQL_UNMANAGED","DATA_SQL_OPERATION_CONFLICT","DATA_SQL_CORRUPT"])
    expect(conversationDeletionStorageBlocked({code,reason:{operationId:requestId}},requestId)).toBe(true);
  for(const code of ["DATA_SQL_COMMIT_UNKNOWN","DATA_SQL_RECOVERY_REQUIRED","DATA_SQL_UNAVAILABLE"])
    expect(conversationDeletionStorageBlocked({code,reason:{operationId:requestId}},requestId)).toBe(false);
});
it("reports completed workspace cleanup separately from an unknown SQL commit", () => {
  const message=conversationDeletionDiagnostic({code:"DATA_OPERATION_PARTIAL",reason:{operationId:requestId,phase:"delete-publication",safeCause:"delete_not_archived_after_workspace_cleanup"}},requestId,{force:true})!;
  expect(message).toContain("工作区文件清理已完成");expect(message).toContain("DATA_OPERATION_PARTIAL");
  expect(message).not.toContain("可能已经执行");expect(message).not.toContain("DATA_SQL_COMMIT_UNKNOWN");
});
