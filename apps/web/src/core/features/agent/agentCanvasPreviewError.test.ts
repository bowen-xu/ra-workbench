import { expect, it } from "vitest";
import { agentCanvasPreviewError } from "./agentCanvasPreviewError";

it("explains a definite fixture capacity failure without suggesting an uncertain retry", () => {
  const message = agentCanvasPreviewError("AGENT_PREVIEW_FIXTURE_CAPACITY");
  expect(message).toContain("预览场景已达上限");
  expect(message).toContain("AGENT_PREVIEW_FIXTURE_CAPACITY");
  expect(message).not.toContain("无法确认预览操作结果");
});

it("retains native string codes and transport code prefixes without their private messages", () => {
  expect(agentCanvasPreviewError("AGENT_PREVIEW_SYNTHETIC_IO_ACK_LOST")).toContain("AGENT_PREVIEW_SYNTHETIC_IO_ACK_LOST");
  const text = agentCanvasPreviewError(new Error("agent_preview_stale_response: private request body"));
  expect(text).toContain("agent_preview_stale_response");
  expect(text).not.toContain("private request body");
});

it.each(["conflict", "notFound", "unavailable"])("retains the exact native enum diagnostic %s", code => {
  expect(agentCanvasPreviewError(code)).toContain(`（${code}）`);
});

it("preserves available status and operation identities but omits arbitrary payloads", () => {
  const operationId = "ec627c80-dcb0-4b62-9702-16855f7c8fdb";
  const text = agentCanvasPreviewError({ code: "CANVAS_FIXTURE_LOCATOR_CONFLICT", status: 409, operationId,
    requestId: "private request body", message: "private provider response", body: "private request body" });
  expect(text).toContain("CANVAS_FIXTURE_LOCATOR_CONFLICT");
  expect(text).toContain("HTTP 409");
  expect(text).toContain(`operationId=${operationId}`);
  expect(text).not.toContain("private");
});

it("does not fabricate a status or expose unknown provider exceptions", () => {
  const text = agentCanvasPreviewError({ status: 9999, message: "private provider response", conversationId: "private title" });
  expect(text).not.toMatch(/HTTP|private|conversationId/u);
});
