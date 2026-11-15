import {describe, expect, it} from "vitest";
import {createAgentNoteSubmissionGateway} from "@research-assistant/data/agent";
import {aiAssistanceErrorMessage} from "./aiAssistanceErrorMessage";

const ids = {
  conversationId: "10000000-0000-4000-8000-000000000001",
  requestId: "20000000-0000-4000-8000-000000000002",
  pageId: "30000000-0000-4000-8000-000000000003",
  instanceId: "40000000-0000-4000-8000-000000000004",
  nativeRequestId: "50000000-0000-4000-8000-000000000005",
};
const diagnostic = {code: "plugin_host_exited", message: "文献宿主在回复前退出。",
  pluginId: "literature_library", packageKey: "a".repeat(64), ...ids};

describe("Core Note native preparation diagnostics", () => {
  it("preserves the native rejection through the real Note submission gateway", async () => {
    const gateway = createAgentNoteSubmissionGateway(async () => {throw diagnostic;});
    let received: unknown;
    try {
      await gateway.prepare({conversationId: ids.conversationId, requestId: ids.requestId,
        text: "Synthetic query", context: {pageId: ids.pageId, expectedRevision: 0, blockId: null}});
    } catch (reason) {received = reason;}
    expect(received).toBe(diagnostic);
    const message = aiAssistanceErrorMessage(received);
    for (const value of [...Object.values(ids), diagnostic.packageKey]) expect(message).toContain(value);
  });

  it("keeps the original receipt distinct from native request and exit identity", () => {
    const message = aiAssistanceErrorMessage({...diagnostic, exitCode: 41, signal: 9,
      cleanupError: "process I/O failed (PermissionDenied)", body: "PRIVATE_BODY",
      sql: "PRIVATE_SQL", stderr: "PRIVATE_STDERR", inputPath: "/PRIVATE_PATH"});
    for (const field of [diagnostic.code, diagnostic.message, diagnostic.packageKey,
      ...Object.values(ids), "exitCode=41", "signal=9", "PermissionDenied"]) expect(message).toContain(field);
    expect(message).not.toContain("PRIVATE_");
  });

  it("keeps safe data phase and driver codes without fabricating a host identity", () => {
    const {instanceId: _instance, nativeRequestId: _native, ...receiptOnly} = diagnostic;
    const message = aiAssistanceErrorMessage({...receiptOnly, code: "plugin_data_sql_failed",
      phase: "execute", safeCause: "ConstraintViolation", sqliteCode: "ConstraintViolation",
      sqliteExtendedCode: 1811, resourceId: "literature.retrieval.trace-mark-sent", resourceVersion: 1});
    for (const field of ["plugin_data_sql_failed", "phase=execute", "ConstraintViolation", "1811",
      ids.requestId, "resourceId=literature.retrieval.trace-mark-sent", "resourceVersion=1"]) expect(message).toContain(field);
    expect(message).not.toMatch(/instanceId=|nativeRequestId=|undefined|null/u);
  });

  it("keeps the same diagnostic inside the original Note queue preparation envelope", () => {
    const cause = {...diagnostic, cleanupError: "原生插件输入清理失败：PermissionDenied"};
    const message = aiAssistanceErrorMessage({code: "preparation", reason: cause});
    expect(message).toBe(aiAssistanceErrorMessage(cause));
    expect(message).toContain(ids.requestId);
    expect(message).toContain("原生插件输入清理失败：PermissionDenied");
  });

  it("ignores untyped technical metadata and private cleanup text", () => {
    const message = aiAssistanceErrorMessage({...diagnostic, packageKey: "PRIVATE_PACKAGE",
      instanceId: "PRIVATE_INSTANCE", nativeRequestId: "PRIVATE_REQUEST", phase: "PRIVATE phase",
      safeCause: "PRIVATE\ncause", sqliteCode: "PRIVATE driver", sqliteExtendedCode: 1.5,
      signal: 2 ** 40, cleanupError: "/PRIVATE_PATH/private input"});
    expect(message).toContain(ids.requestId);
    expect(message).not.toContain("PRIVATE");
    expect(message).not.toMatch(/phase=|safeCause=|sqliteCode=|sqliteExtendedCode=|signal=|cleanup=/u);
  });

  it("retains the original generic message contract for an unrelated error shape", () => {
    expect(aiAssistanceErrorMessage({...diagnostic, pluginId: "unrelated"}))
      .toBe(`${diagnostic.message}（错误码：${diagnostic.code}）`);
    expect(aiAssistanceErrorMessage({...diagnostic, requestId: "not-a-uuid"}))
      .toBe(`${diagnostic.message}（错误码：${diagnostic.code}）`);
  });
});
