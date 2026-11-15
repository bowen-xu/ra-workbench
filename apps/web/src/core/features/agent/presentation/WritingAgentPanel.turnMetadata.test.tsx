/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import { WritingAgentPanel, type WritingAgentMessage } from "./WritingAgentPanel";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

it("keeps completed text items without a footer until the turn ends and preserves older response footers", async () => {
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container);
  const messages: readonly WritingAgentMessage[] = [
    { id: "previous", turnId: "previous", turnFinished: true, role: "assistant", content: "上一轮已完成" },
    { id: "request", turnId: "current", turnFinished: false, role: "user", content: "修改摘要" },
    { id: "commentary", turnId: "current", turnFinished: false, role: "assistant", phase: "commentary", content: "工具结果已接收，继续处理。", streaming: false },
    { id: "steer", turnId: "current", turnFinished: false, role: "user", content: "同时检查关键词" },
    { id: "final", turnId: "current", turnFinished: false, role: "assistant", phase: "finalAnswer", content: "已准备修改", streaming: false },
  ];
  const footer = (id: string) => container.querySelector(`[data-agent-message-id="${id}"] .writing-agent-message-meta`);
  try {
    await act(async () => root.render(<WritingAgentPanel open isRunning messages={messages} />));
    expect(footer("previous")).not.toBeNull();
    expect(footer("commentary")).toBeNull(); expect(footer("final")).toBeNull();
    expect(container.querySelector('[data-agent-message-id="commentary"]')?.textContent).toContain("继续处理");
    // A transient idle UI flag cannot substitute for the durable terminal event.
    await act(async () => root.render(<WritingAgentPanel open messages={messages} />));
    expect(footer("final")).toBeNull();
    await act(async () => root.render(<WritingAgentPanel open messages={messages.map(message => ({ ...message, turnFinished: true }))} />));
    expect(footer("commentary")).toBeNull(); expect(footer("final")).not.toBeNull();
    expect(footer("previous")).not.toBeNull();
  } finally { await act(async () => root.unmount()); container.remove(); }
});

it("renders skill loading through the shared tool activity with no assistant message metadata", async () => {
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container);
  const messages: readonly WritingAgentMessage[] = [
    { id: "load", turnId: "turn", role: "assistant", kind: "tool-call", toolName: "research_load_skill", content: '{"identity":{"source_id":"review"}}' },
    { id: "loaded", turnId: "turn", role: "assistant", kind: "tool-result", toolName: "research_load_skill", content: '{"skill":{"name":"模拟写作技能","body":"instructions"}}' },
  ];
  try {
    await act(async () => root.render(<WritingAgentPanel open messages={messages} />));
    const activity = container.querySelector('[data-agent-activity-single]');
    expect(activity?.querySelector(".tool-row-compact")?.textContent).toContain("已读取 模拟写作技能 技能");
    expect(activity?.querySelector('[data-tool-status="success"]')).not.toBeNull();
    expect(activity?.querySelector('pre[aria-label="调用结果"]')).toBeNull();
    expect(activity?.textContent).not.toContain("instructions");
    expect(container.querySelector(".writing-agent-message.assistant")).toBeNull();
    expect(container.querySelector(".writing-agent-message-meta")).toBeNull();
    // Old persisted results without display names still use the same activity.
    await act(async () => root.render(<WritingAgentPanel open messages={[messages[0]!, { ...messages[1]!, content: '{"skill":{"body":"legacy"}}' }]} />));
    expect(container.querySelector('[data-agent-activity-single] .tool-row-compact')?.textContent).toBe("已读取 review 技能");
    expect(container.querySelector('[data-agent-activity-group]')).toBeNull();
  } finally { await act(async () => root.unmount()); container.remove(); }
});
