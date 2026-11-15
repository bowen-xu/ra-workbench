import { expect, it } from "vitest";
import { queueReadDiagnostic } from "./queueReadDiagnostic";

it("reports safe native and transport diagnostics and the frozen scope", () => {
  const scope = {conversationId: "11111111-1111-4111-8111-111111111111", manuscriptId: "22222222-2222-4222-8222-222222222222"};
  const message = queueReadDiagnostic({scope, cause: {code: "AGENT_PREVIEW_SYNTHETIC_IO_UNAVAILABLE", status: 503,
    requestId: "33333333-3333-4333-8333-333333333333", authorization: "Bearer private"}});
  expect(message).toContain("code=AGENT_PREVIEW_SYNTHETIC_IO_UNAVAILABLE");
  expect(message).toContain(`C=${scope.conversationId}`);
  expect(message).toContain(`M=${scope.manuscriptId}`);
  expect(message).toContain("HTTP=503");
  expect(message).toContain("requestId=33333333-3333-4333-8333-333333333333");
  expect(message).not.toContain("Bearer private");
});

it("does not reveal arbitrary messages or legacy resource names", () => {
  const message = queueReadDiagnostic({scope: {conversationId: "private conversation", paperId: "private paper"},
    cause: new Error("private request content and credentials")});
  expect(message).toContain("code=QUEUE_READ_FAILED");
  expect(message).not.toContain("private");
});

it("preserves safe network, operating system and stack information without paths", () => {
  const cause = new TypeError("Failed to fetch") as TypeError & {osError: number};
  cause.osError = 35;
  cause.stack = "TypeError: Failed to fetch\n    at load (/private/user/queue.ts:42:7)";
  const message = queueReadDiagnostic({scope: {kind: "paper", paperId: "11111111-1111-4111-8111-111111111111"}, cause});
  expect(message).toContain("transport=network_error"); expect(message).toContain("osError=35");
  expect(message).toContain("stack=queue.ts:42:7"); expect(message).toContain("scope=paper");
  expect(message).not.toContain("/private/user");
});
