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
const diagnostic = {code: "plugin_host_response_mismatch", message: "文献回复与本次请求不符。",
  pluginId: "literature_library", packageKey: "a".repeat(64), exitCode: 1, ...ids};
const termination = {operation: "group_terminate", kind: "PermissionDenied", rawOsError: 1};

describe("confirmed native termination diagnostics", () => {
  it("preserves the fixed original errno and both receipt and host identities through the Note gateway", async () => {
    const cause = {...diagnostic, terminationError: termination};
    const gateway = createAgentNoteSubmissionGateway(async () => {throw cause;});
    let received: unknown;
    try {
      await gateway.prepare({conversationId: ids.conversationId, requestId: ids.requestId,
        text: "Synthetic query", context: {pageId: ids.pageId, expectedRevision: 0, blockId: null}});
    } catch (reason) {received = reason;}
    expect(received).toBe(cause);
    const message = aiAssistanceErrorMessage(received);
    for (const value of [diagnostic.code, diagnostic.message, diagnostic.packageKey,
      ...Object.values(ids), "exitCode=1", "operation=group_terminate", "kind=PermissionDenied", "rawOsError=1"]) expect(message).toContain(value);
    expect(message).toContain("终止阶段诊断（已确认完成收尾）");
    expect(message).not.toContain("cleanup=");
    expect(aiAssistanceErrorMessage({code: "preparation", reason: cause})).toBe(message);
  });

  it("keeps the exact legacy message when the optional diagnostic is absent", () => {
    const expected = aiAssistanceErrorMessage(diagnostic);
    expect(aiAssistanceErrorMessage({...diagnostic, terminationError: undefined})).toBe(expected);
    expect(aiAssistanceErrorMessage({...diagnostic, terminationError: null})).toBe(expected);
    expect(expected).not.toContain("终止阶段诊断");
  });

  it.each([
    {operation: "/PRIVATE_PATH", kind: "PermissionDenied", rawOsError: 1},
    {...termination, operation: "group_cleanup_after_wait"},
    {...termination, operation: "group_terminate\nPRIVATE_BODY"},
    {...termination, kind: "/PRIVATE_PATH"},
    {...termination, kind: "Other"},
    {...termination, rawOsError: "1"},
    {...termination, rawOsError: null},
    {...termination, rawOsError: 0},
    {...termination, rawOsError: -1},
    {...termination, rawOsError: 2},
    {...termination, rawOsError: 1.5},
    {...termination, rawOsError: Number.NaN},
    {...termination, rawOsError: Number.POSITIVE_INFINITY},
    {...termination, rawOsError: 2 ** 40},
    {...termination, path: "/PRIVATE_PATH"},
    {...termination, stderr: "PRIVATE_STDERR"},
    {operation: "group_terminate", kind: "PermissionDenied"},
    [termination],
    "PRIVATE_STDERR",
  ])("ignores a malformed or extended termination DTO %# without changing the original diagnostic", value => {
    const message = aiAssistanceErrorMessage({...diagnostic, terminationError: value});
    expect(message).toBe(aiAssistanceErrorMessage(diagnostic));
    expect(message).not.toContain("PRIVATE");
    expect(message).not.toContain("终止阶段诊断");
  });

  it("rejects inherited or non-JSON extra fields and never relaxes the cleanup allowlist", () => {
    const symbol = {...termination, [Symbol("private")]: "/PRIVATE_PATH"};
    const hidden = Object.defineProperty({...termination}, "path", {value: "/PRIVATE_PATH"});
    for (const value of [Object.create(termination), symbol, hidden]) {
      expect(aiAssistanceErrorMessage({...diagnostic, terminationError: value}))
        .toBe(aiAssistanceErrorMessage(diagnostic));
    }
    const message = aiAssistanceErrorMessage({...diagnostic, terminationError: termination,
      cleanupError: "/PRIVATE_PATH", stderr: "PRIVATE_STDERR", body: "PRIVATE_BODY"});
    expect(message).toContain("终止阶段诊断（已确认完成收尾）");
    expect(message).not.toContain("PRIVATE");
    expect(message).not.toContain("cleanup=");
  });
});
