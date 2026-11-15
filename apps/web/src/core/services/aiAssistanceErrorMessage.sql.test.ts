import { describe, expect, it } from "vitest";
import fixture from "../../../../../tests/fixtures/agent-sql-immediate-unknown-v1.json";
import { aiAssistanceErrorMessage } from "./aiAssistanceErrorMessage";

describe("SQL operation diagnostics", () => {
  it("retains writing preparation and exact Native source identities", () => {
    const id = fixture.reason.reason.operationId;
    const message = aiAssistanceErrorMessage({code: "storage", reason: {
      conversationId: id, manuscriptId: id, requestId: id,
      code: "DATA_SQL_OPERATION_CONFLICT", reason: {operationId: id, phase: "writing-prepare-materialize", safeCause: "writing_context_changed"},
      source: {kind: "native", packageKey: "a".repeat(64), instanceId: id, requestId: id,
        body: "private manuscript", authorization: "private credential"},
    }});
    for (const part of ["writing_context_changed", `conversationId=${id}`, `manuscriptId=${id}`, `requestId=${id}`,
      `packageKey=${"a".repeat(64)}`, `instanceId=${id}`, `nativeRequestId=${id}`]) expect(message).toContain(part);
    expect(message).not.toMatch(/private|authorization/u);
  });

  it("keeps the original Core data resource failure and filters unrelated payloads", () => {
    const message = aiAssistanceErrorMessage({code: "storage", reason: {
      code: "DATA_SQL_UNAVAILABLE", reason: {phase: "writing-prepare-materialize", safeCause: "writing_context_source_unavailable"},
      source: {kind: "data", error: {code: "plugin_data_statement_failed", resourceId: "writing.projects.context-current-file",
        phase: "read", safeCause: "sqlite_read_failed", sqliteCode: "AuthorizationForStatementDenied", sqliteExtendedCode: 23,
        sql: "private SQL", authorization: "private credential"}},
    }});
    for (const token of ["writing_context_source_unavailable", "plugin_data_statement_failed", "writing.projects.context-current-file", "sqlite_read_failed", "23"]) expect(message).toContain(token);
    expect(message).not.toMatch(/private|authorization/u);
  });

  it("shows finite writing Native failure with both original and stage request identities", () => {
    const id = fixture.reason.reason.operationId;
    const message = aiAssistanceErrorMessage({pluginId: "writing_projects", code: "plugin_host_exited", message: "写作宿主已退出。",
      conversationId: id, manuscriptId: id, operationId: id, packageKey: "b".repeat(64), instanceId: id, requestId: id,
      exitCode: 43, rawStderr: "private body", authorization: "private credential"});
    for (const token of ["写作宿主已退出", "plugin_host_exited", `conversationId=${id}`, `manuscriptId=${id}`, `operationId=${id}`,
      `instanceId=${id}`, `nativeRequestId=${id}`, "exitCode=43"]) expect(message).toContain(token);
    expect(message).not.toMatch(/private|authorization|rawStderr/u);
  });

  it("preserves the shared native unknown receipt without rounding its fence", () => {
    const message = aiAssistanceErrorMessage(fixture);
    for (const part of ["数据提交结果尚未确认", "DATA_SQL_COMMIT_UNKNOWN",
      fixture.reason.reason.operationId, fixture.reason.reason.epoch,
      "sqlFence=9007199254740993", "phase=commit", "cause=sql_commit_result_unknown",
      "SQLite=SystemIoFailure", "sqliteExtendedCode=778"]) expect(message).toContain(part);
    expect(message).not.toContain("9007199254740992");
  });

  it("retains the available identity when a worker has no epoch or SQL fence", () => {
    const message = aiAssistanceErrorMessage({code: "storage", reason: {
      code: "DATA_SQL_COMMIT_UNKNOWN", reason: {operationId: fixture.reason.reason.operationId,
        epoch: null, sqlFence: null, phase: "authorization-worker",
        safeCause: "authorization_worker_result_unknown", sqliteCode: null, sqliteExtendedCode: null},
    }});
    expect(message).toContain(fixture.reason.reason.operationId);
    expect(message).toContain("authorization_worker_result_unknown");
    expect(message).not.toMatch(/epoch=|sqlFence=|SQLite=|null/u);
  });

  it("shows a paper preparation failure using the shared native storage envelope", () => {
    const message = aiAssistanceErrorMessage({code: "storage", reason: {
      code: "DATA_SQL_COMMIT_UNKNOWN", reason: {operationId: fixture.reason.reason.operationId,
        epoch: fixture.reason.reason.epoch, sqlFence: "9007199254740993", phase: "paper-prepare",
        safeCause: "sql_commit_outcome_unknown", sqliteCode: "ConstraintViolation", sqliteExtendedCode: 787},
    }});
    for (const part of ["DATA_SQL_COMMIT_UNKNOWN", fixture.reason.reason.operationId,
      "sqlFence=9007199254740993", "paper-prepare", "sql_commit_outcome_unknown", "ConstraintViolation", "787"]) expect(message).toContain(part);
  });

  it("displays only bounded technical fields from the known native envelope", () => {
    const message = aiAssistanceErrorMessage({code: "storage", reason: {
      code: "DATA_SQL_RECOVERY_REQUIRED", reason: {
        operationId: "private manuscript body", epoch: "not-a-uuid", sqlFence: "18446744073709551616",
        phase: "select * from private", safeCause: "private body\nother text",
        sqliteCode: "private query text", sqliteExtendedCode: 2.5,
        sql: "private SQL", body: "private body", authorization: "private credential",
      },
    }});
    expect(message).toContain("DATA_SQL_RECOVERY_REQUIRED");
    expect(message).not.toMatch(/private|select|operationId=|epoch=|sqlFence=|phase=|cause=|SQLite=/u);
  });

  it("keeps the old Error and structured-message contracts", () => {
    expect(aiAssistanceErrorMessage(new Error("原有错误"))).toBe("原有错误");
    expect(aiAssistanceErrorMessage({code: "authentication", message: "认证失败"}))
      .toBe("认证失败（错误码：authentication）");
    expect(aiAssistanceErrorMessage("原有文字")).toBe("原有文字");
  });

  it.each(["0", "18446744073709551615"])("keeps the exact decimal boundary %s", sqlFence => {
    const value = {...fixture.reason, reason: {...fixture.reason.reason, sqlFence}};
    expect(aiAssistanceErrorMessage(value)).toContain(`sqlFence=${sqlFence}`);
  });
});

describe("queued writing source diagnostics", () => {
  const conversationId = "10000000-0000-0000-0000-000000000001";
  const manuscriptId = "10000000-0000-0000-0000-000000000002";
  const operationId = "10000000-0000-0000-0000-000000000003";
  const inputId = "10000000-0000-0000-0000-000000000004";
  const requestId = "10000000-0000-0000-0000-000000000005";
  const scope = {conversationId, manuscriptId, operationId, inputId};

  it.each(["queue", "preparation"])("retains %s classification and exact source rejection", code => {
    const message = aiAssistanceErrorMessage({code, reason: code === "queue" ? {code: "unavailable"} : "unavailable", writingSource: {...scope,
      source: {kind: "native", packageKey: "c".repeat(64), instanceId: inputId, requestId,
        rejection: "conflict", body: "private manuscript", authorization: "private credential"},
    }});
    for (const token of [`错误码：${code}`, "reason=unavailable", `conversationId=${conversationId}`,
      `manuscriptId=${manuscriptId}`, `operationId=${operationId}`, `inputId=${inputId}`,
      `nativeRequestId=${requestId}`, "source.rejection=conflict", `packageKey=${"c".repeat(64)}`]) expect(message).toContain(token);
    expect(message).not.toMatch(/private|authorization/u);
  });

  it("retains original SQL diagnostics together with queued input provenance", () => {
    const message = aiAssistanceErrorMessage({code: "storage", reason: {
      code: "DATA_SQL_OPERATION_CONFLICT", reason: {operationId, phase: "queue-prepare-context", safeCause: "writing_context_changed"},
    }, writingSource: {...scope, source: {kind: "native", instanceId: inputId, requestId, rejection: "conflict"}}});
    for (const token of ["DATA_SQL_OPERATION_CONFLICT", "queue-prepare-context", "writing_context_changed",
      `operationId=${operationId}`, `inputId=${inputId}`, `nativeRequestId=${requestId}`]) expect(message).toContain(token);
  });

  it("retains queued transport error details without replacing the caller operation", () => {
    const message = aiAssistanceErrorMessage({code: "preparation", reason: "unavailable", writingSource: {...scope,
      source: {kind: "nativeFailure", error: {code: "plugin_host_exited", pluginId: "writing_projects",
        message: "写作宿主已退出。", requestId, exitCode: 43, rawStderr: "private body", authorization: "private credential"}},
    }});
    for (const token of ["写作宿主已退出", "plugin_host_exited", `conversationId=${conversationId}`,
      `operationId=${operationId}`, `inputId=${inputId}`, `nativeRequestId=${requestId}`, "exitCode=43"]) expect(message).toContain(token);
    expect(message).not.toMatch(/private|authorization/u);
  });

  it("ignores invalid source scope and unknown source properties", () => {
    const message = aiAssistanceErrorMessage({code: "queue", reason: "unavailable", writingSource: {...scope,
      operationId: "private manuscript", source: {kind: "nativeFailure", error: {message: "private data"}},
    }});
    expect(message).not.toMatch(/private|writingSource|inputId=/u);
    const valid = aiAssistanceErrorMessage({code: "queue", reason: "unavailable", writingSource: {...scope,
      source: {kind: "native", rejection: "private manuscript", requestId: "private request", sql: "private SQL"},
    }});
    expect(valid).not.toMatch(/private|rejection=|nativeRequestId=/u);
  });
});
