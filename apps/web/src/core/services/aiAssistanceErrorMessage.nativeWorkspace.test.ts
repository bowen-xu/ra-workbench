// @vitest-environment happy-dom
import {expect, it} from "vitest";
import {aiAssistanceErrorMessage} from "./aiAssistanceErrorMessage";

it("keeps native workspace source and fixed SQL identities through the existing preparation envelope", () => {
  const scope = {
    conversationId: "10000000-0000-0000-0000-000000000001",
    manuscriptId: "10000000-0000-0000-0000-000000000002",
    operationId: "10000000-0000-0000-0000-000000000003",
    requestId: "10000000-0000-0000-0000-000000000004",
    instanceId: "10000000-0000-0000-0000-000000000005",
  };
  for (const code of ["plugin_host_source_rejected", "storage_error"]) {
    const message = aiAssistanceErrorMessage({
      ...scope, code, pluginId: "writing_projects", message: "工作副本来源读取失败。",
      packageKey: "a".repeat(64), privateResponse: "private body",
      dataError: {code: "plugin_data_read_failed", resourceId: "writing.projects.files",
        resourceVersion: 1, packageKey: "a".repeat(64), phase: "read",
        safeCause: "sqlite_query_failed", sqliteCode: "Unknown", sqliteExtendedCode: 1,
        rawSql: "private SQL", arguments: "private arguments"},
    });
    for (const token of [code, `conversationId=${scope.conversationId}`, `manuscriptId=${scope.manuscriptId}`,
      `operationId=${scope.operationId}`, `nativeRequestId=${scope.requestId}`, `instanceId=${scope.instanceId}`,
      `packageKey=${"a".repeat(64)}`, "resourceId=writing.projects.files", "resourceVersion=1",
      "source.sqliteCode=Unknown", "source.sqliteExtendedCode=1"]) expect(message).toContain(token);
    expect(message).not.toMatch(/private|rawSql|arguments|privateResponse/u);
  }
});
