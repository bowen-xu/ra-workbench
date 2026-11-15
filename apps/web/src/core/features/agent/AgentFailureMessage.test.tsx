/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { WritingAgentTranscript } from "./presentation/WritingAgentTranscript";
import type { WritingAgentMessage } from "./presentation/WritingAgentMessage";

let host: HTMLDivElement, root: Root;
const failure: WritingAgentMessage = { id: "failed/failure", turnId: "failed", role: "assistant", failure: true,
  content: "Agent 请求失败：工具请求未通过校验。\n错误码：tool_request_rejected\n错误编号（turnId）：failed",
  failureDiagnosticText: '{"conversationId":"conversation","turnId":"failed"}', turnFinished: true, turnOutcome: "failed" };
beforeEach(() => {
  (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText: vi.fn().mockResolvedValue(undefined) } });
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); });

it("copies diagnostics directly inside an old failure even after a newer turn, without developer mode", async () => {
  await act(async () => root.render(<WritingAgentTranscript isRunning={false} messages={[failure,
    { id: "next/request", turnId: "next", role: "user", content: "继续" },
    { id: "next/reply", turnId: "next", role: "assistant", content: "成功", turnFinished: true },
  ]} />));
  const alert = host.querySelector('[role="alert"].failure')!;
  const button = alert.querySelector<HTMLButtonElement>('[aria-label="复制诊断信息"]')!;
  expect(button.textContent).toBe("复制诊断信息");
  expect(button.closest(".writing-agent-message-meta")).toBeNull();
  await act(async () => button.click());
  expect(navigator.clipboard.writeText).toHaveBeenCalledWith(`${failure.content}\n\n运行诊断：\n${failure.failureDiagnosticText}`);
  expect(button.textContent).toBe("已复制");
});

it("copies legacy failures without inventing missing runtime data", async () => {
  const { failureDiagnosticText: _metadata, ...legacy } = failure;
  await act(async () => root.render(<WritingAgentTranscript isRunning={false} messages={[legacy]} />));
  await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="复制诊断信息"]')!.click());
  expect(navigator.clipboard.writeText).toHaveBeenCalledWith(legacy.content);
});

it("shows failure and permits retry when both clipboard paths fail", async () => {
  vi.mocked(navigator.clipboard.writeText).mockRejectedValue(new Error("unavailable"));
  Object.defineProperty(document, "execCommand", { configurable: true, value: vi.fn().mockReturnValue(false) });
  await act(async () => root.render(<WritingAgentTranscript isRunning={false} messages={[failure]} />));
  const button = host.querySelector<HTMLButtonElement>('[aria-label="复制诊断信息"]')!;
  await act(async () => button.click());
  expect(button.textContent).toBe("复制失败，请重试");
  expect(button.disabled).toBe(false);
  vi.mocked(navigator.clipboard.writeText).mockResolvedValue(undefined);
  await act(async () => button.click());
  expect(button.textContent).toBe("已复制");
});
