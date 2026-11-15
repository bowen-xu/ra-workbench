import { expect, it } from "vitest";
import { agentFailureCode, agentFailureReason } from "./agentFailureReason";

it.each([
  ["model server overloaded", "负载过高"],
  ["model context window exceeded", "上下文容量"],
  ["model usage limit reached", "额度已耗尽"],
  ["model request timed out", "响应超时"],
  ["model connection failed", "无法建立"],
  ["model response read failed", "传输中断"],
  ["unexpected model HTTP status 503", "HTTP 503（服务不可用）"],
  ["model response ended before completion", "未收到完成标记"],
  ["unsupported model response type", "更新响应适配"],
  ["model authentication rejected with HTTP 401", "HTTP 401"],
  ["model authentication rejected with HTTP 403", "HTTP 403"],
  ["model rate limited with HTTP 429", "HTTP 429"],
])("explains the known cause of %s", (error, cause) => expect(agentFailureReason(error)).toContain(cause));
it("does not guess the cause of generic or legacy errors", () => {
  expect(agentFailureReason("model temporarily unavailable")).toContain("未提供更具体的原因");
  expect(agentFailureReason("legacy safe error")).toBe("legacy safe error");
  expect(agentFailureReason("model capability unavailable")).not.toContain("所选模型不支持");
});
it("provides a reproducible code for native errors and a fallback for legacy diagnostics",()=>{
  expect(agentFailureCode("model authentication rejected with HTTP 403")).toBe("model_authentication_rejected_with_http_403");
  expect(agentFailureCode("旧错误信息")).toBe("agent_failure");
});
it("keeps the stable rejection code for detailed pre-execution diagnostics", () => {
  const error = 'tool request rejected: {"code":"duplicate_call_id","toolName":"research_read_file"}';
  expect(agentFailureCode(error)).toBe("tool_request_rejected");
  expect(agentFailureReason(error)).toContain("执行前校验");
});
it("keeps unknown tool outcomes distinct from retryable failures", () => {
  const error = "tool_outcome_unknown: tool execution outcome requires recovery";
  expect(agentFailureCode(error)).toBe("tool_outcome_unknown");
  expect(agentFailureReason(error)).toContain("先核对原操作");
});
