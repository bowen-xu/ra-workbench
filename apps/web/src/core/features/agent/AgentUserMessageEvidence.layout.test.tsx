/** @vitest-environment happy-dom */
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { expect, it } from "vitest";
import { agentDraftInput, type AgentEvidenceInput } from "@research-assistant/core/agent";
import { WritingAgentTranscript } from "./presentation/WritingAgentTranscript";
import { AgentPendingSubmissionBubble } from "./AgentPendingSubmissionBubble";
import { AgentPendingSteeringBubbles } from "./AgentPendingSteeringBubbles";

const evidence: AgentEvidenceInput = {type: "annotation", source: "source://notes/note", title: "来源", text: "引用内容", comment: "批注内容"};
const input = agentDraftInput({text: "用户消息", bindings: [], evidence: [evidence]});
const message = {id: "turn/user", conversationId: "conversation", turnId: "turn", role: "user" as const,
  content: "用户消息", streaming: false, userInput: input};

it.each<[string, ReactNode]>([
  ["saved", <WritingAgentTranscript messages={[message]} isRunning={false} />],
  ["pending submission", <AgentPendingSubmissionBubble messages={[]} pending={{id: "pending", conversationId: "conversation", text: "用户消息", userInput: input}} />],
  ["pending steering", <AgentPendingSteeringBubbles messages={[]} conversationId="conversation" direct={[{conversationId: "conversation", turnId: "turn", inputId: "user", text: "用户消息", userInput: input}]} />],
])("places the read-only annotation above the %s bubble and keeps it expandable", async (_kind, tree) => {
  (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => root.render(tree));
    const article = host.querySelector<HTMLElement>(".writing-agent-message.user")!;
    const badge = article.querySelector<HTMLButtonElement>(".agent-evidence-disclosure")!;
    const bubble = article.querySelector<HTMLElement>(".writing-agent-message-bubble")!;
    expect(badge.textContent).toBe("1 条批注");
    expect(bubble.contains(badge)).toBe(false);
    expect(badge.compareDocumentPosition(bubble) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(bubble.textContent).toContain("用户消息");
    expect(article.querySelector('[aria-label="清除全部批注"]')).toBeNull();
    await act(async () => badge.click());
    expect(badge.getAttribute("aria-expanded")).toBe("true");
    expect(document.querySelector('.agent-evidence-popover')?.textContent).toContain("批注内容");
  } finally { await act(async () => root.unmount()); host.remove(); }
});
