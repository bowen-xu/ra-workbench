import {describe, expect, it} from "vitest";
import {aiAssistanceErrorMessage} from "./aiAssistanceErrorMessage";
import {decodeAgentSqlImmediateFailure, isAgentSqlImmediateUnknown} from "@research-assistant/core/agent";

const conversation = "10000000-0000-4000-8000-000000000001";
const request = "10000000-0000-4000-8000-000000000002";
const instance = "10000000-0000-4000-8000-000000000003";
const nativeRequest = "10000000-0000-4000-8000-000000000004";
function failure(source: unknown, code = "DATA_SQL_UNAVAILABLE") {
  return {
    code: "storage",
    reason: {code, reason: {operationId: request, phase: "context-prepare-source", safeCause: "context_prepare_source_unavailable"}},
    contextSource: {conversationId: conversation, requestId: request, source},
  };
}

describe("ordinary context preparation provenance", () => {
  it("retains actual Core and Native identities without manufacturing a manuscript identity", () => {
    const value = failure({kind: "nativeFailure", error: {
      pluginId: "writing_projects", code: "plugin_host_exited", message: "写作来源宿主已退出。",
      packageKey: "a".repeat(64), instanceId: instance, requestId: nativeRequest, exitCode: 43,
      rawStderr: "private body", authorization: "private credential",
    }});
    const message = aiAssistanceErrorMessage(value);
    for (const part of ["DATA_SQL_UNAVAILABLE", "context_prepare_source_unavailable", `conversationId=${conversation}`,
      `requestId=${request}`, `instanceId=${instance}`, `nativeRequestId=${nativeRequest}`, `packageKey=${"a".repeat(64)}`, "exitCode=43"]) {
      expect(message).toContain(part);
    }
    expect(message).not.toMatch(/manuscriptId=|inputId=|private|authorization|rawStderr/u);
    expect(decodeAgentSqlImmediateFailure(value)?.code).toBe("DATA_SQL_UNAVAILABLE");
  });

  it("keeps SQL uncertainty classification together with consumed catalog diagnostics", () => {
    const value = failure({kind: "nativeFailure", error: {
      pluginId: "writing_projects", code: "writing_file_kind_policy_invalid", message: "固定路径规则读取失败。",
      packageKey: "b".repeat(64), dataError: {code: "writing_scope_source_unavailable", resourceId: "writing.projects.file-kinds",
        resourceVersion: 1, phase: "resource", safeCause: "resource_hash_mismatch", sqliteCode: "AuthorizationForStatementDenied", sqliteExtendedCode: 23},
    }}, "DATA_SQL_RECOVERY_REQUIRED");
    const message = aiAssistanceErrorMessage(value);
    for (const part of ["DATA_SQL_RECOVERY_REQUIRED", "writing_file_kind_policy_invalid", "writing_scope_source_unavailable",
      "writing.projects.file-kinds", "resourceVersion=1", "resource_hash_mismatch", "sqliteExtendedCode=23"]) {
      expect(message).toContain(part);
    }
    expect(isAgentSqlImmediateUnknown(value)).toBe(true);
  });

  it("copies only bounded fields from an ordered pure rejection or data failure", () => {
    const pure = aiAssistanceErrorMessage(failure({kind: "native", packageKey: "c".repeat(64),
      instanceId: instance, requestId: nativeRequest, rejection: "conflict", body: "private document"}));
    expect(pure).toContain("source.rejection=conflict");
    expect(pure).toContain(`nativeRequestId=${nativeRequest}`);
    const data = aiAssistanceErrorMessage(failure({kind: "data", error: {
      code: "plugin_data_statement_failed", phase: "read", safeCause: "sqlite_read_failed",
      resourceId: "writing.projects.context-current-file", resourceVersion: 1, packageKey: "d".repeat(64),
      body: "private document", authorization: "private credential",
    }}));
    expect(data).toContain("writing.projects.context-current-file");
    expect(data).toContain("sqlite_read_failed");
    expect(pure + data).not.toMatch(/private|authorization/u);
  });

  it("retains the original SQL diagnosis when additional provenance is malformed", () => {
    const value = failure({kind: "nativeFailure", error: {code: "plugin_host_exited", message: "private body"}});
    value.contextSource.requestId = "private request content";
    const message = aiAssistanceErrorMessage(value);
    expect(message).toContain("DATA_SQL_UNAVAILABLE");
    expect(message).not.toMatch(/private|contextSource|requestId=/u);
  });
});
