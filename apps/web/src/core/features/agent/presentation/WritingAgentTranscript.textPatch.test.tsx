/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { WritingAgentTranscript } from "./WritingAgentTranscript";
import type { WritingAgentMessage } from "./WritingAgentMessage";

vi.mock("../AgentTextChangeCard", async importOriginal => {
  const original = await importOriginal<typeof import("../AgentTextChangeCard")>();
  return { ...original, AgentTextChangeCard: ({ content }: { content: string }) => <section aria-label="本轮变更">{content}</section> };
});
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
function result(turnId: string, id: string, status = "proposed", streaming = false): WritingAgentMessage {
  return { id, turnId, role: "assistant", kind: "tool-result", toolName: "research_apply_patch", conversationId: "conversation", streaming,
    content: JSON.stringify({ targets: [{ path: "source://notes/note", name: "笔记", status, message: "result", proposalId: "proposal" }] }) };
}
async function setup() {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  return { host, render: async (messages: readonly WritingAgentMessage[], isRunning = false, turnAttachment?: (turn: string) => React.ReactNode,
    turnAttachmentPaths?: (turn: string) => ReadonlySet<string>) => {
    await act(async () => root.render(<WritingAgentTranscript messages={messages} isRunning={isRunning} {...(turnAttachment ? { turnAttachment } : {})}
      {...(turnAttachmentPaths ? {turnAttachmentPaths} : {})} />));
  }, close: async () => { await act(async () => root.unmount()); host.remove(); } };
}
it("attaches one deduplicated patch summary after the final response and outside collapsed tools", async () => {
  const view = await setup();
  try {
    await view.render([{ id: "user", turnId: "one", role: "user", content: "修改" }, result("one", "first", "unknown"), result("one", "second"),
      { id: "final", turnId: "one", role: "assistant", content: "修改已就绪", turnFinished: true }]);
    const card = view.host.querySelector('[aria-label="本轮变更"]');
    const response = view.host.querySelector('[data-agent-message-id="final"]');
    expect(card?.closest("article")).toBe(response);
    expect(response?.querySelector(".writing-agent-message-bubble")?.lastElementChild).toBe(card);
    expect(view.host.querySelector(".writing-agent-turn-process")?.contains(card)).toBe(false);
    expect(view.host.querySelectorAll('[aria-label="本轮变更"]')).toHaveLength(1);
    expect(card?.textContent).toContain('"status":"proposed"');
    expect(card?.textContent).not.toContain('"status":"unknown"');
    expect(response?.lastElementChild?.classList.contains("writing-agent-message-meta")).toBe(true);
  } finally { await view.close(); }
});
it("keeps the file owner summary without duplicating applied file receipts, retaining mixed note and failure results", async () => {
  const view = await setup();
  try {
    const fileResult = { ...result("one", "file", "applied"), content: JSON.stringify({ targets: [{ path: "source://file", status: "applied", message: "copy updated" }] }) };
    const messages: WritingAgentMessage[] = [{ id: "user", turnId: "one", role: "user", content: "修改" }, fileResult,
      { id: "reply", turnId: "one", role: "assistant", content: "完成", turnFinished: true }];
    const attachment = () => <section aria-label="原文件摘要">Edited files</section>;
    await view.render(messages, false, attachment, () => new Set(["source://file"]));
    expect(view.host.querySelector('[aria-label="本轮变更"]')).toBeNull();
    expect(view.host.querySelector('[data-agent-message-id="reply"] .writing-agent-message-bubble')?.lastElementChild?.getAttribute("aria-label")).toBe("原文件摘要");
    const receipt = { ...result("one", "mixed"), content: JSON.stringify({ targets: [
      { path: "source://file", status: "applied", name: "file", message: "copy updated" },
      { path: "source://notes/note", status: "proposed", name: "note", proposalId: "proposal", message: "candidate updated" },
      { path: "source://library/paper/portrait", status: "applied", name: "画像", changeId: "change", message: "saved", structuredResource: { resourceId: "portrait", revision: 1 } },
      { path: "source://notes/denied", status: "failed", name: "denied", message: "conflict" },
    ] }) };
    await view.render([messages[0]!, receipt, messages[2]!], false, attachment, () => new Set(["source://file"]));
    const card = view.host.querySelector('[aria-label="本轮变更"]');
    expect(card?.textContent).not.toContain('"path":"source://file"');
    expect(card?.textContent).toContain('"resourceId":"portrait"');
    expect(card?.textContent).toContain('"changeId":"change"');
    expect(card?.textContent).toContain('"status":"proposed"');
    expect(card?.textContent).toContain('"status":"failed"');
  } finally { await view.close(); }
});
it("retains each patch receipt until its own turn and file are covered by a visible summary", async () => {
  const view = await setup();
  try {
    const first = { ...result("one", "file-one", "applied"), content: JSON.stringify({targets:[
      {path:"source://writings/project/a.tex",status:"applied",message:"first"},
      {path:"source://writings/project/b.tex",status:"applied",message:"second"},
    ]}) };
    const second = { ...result("two", "file-two", "applied"), content: JSON.stringify({targets:[
      {path:"source://writings/project/a.tex",status:"applied",message:"later turn"},
    ]}) };
    const messages: WritingAgentMessage[] = [
      {id:"u1",turnId:"one",role:"user",content:"first"}, first, {id:"r1",turnId:"one",role:"assistant",content:"done",turnFinished:true},
      {id:"u2",turnId:"two",role:"user",content:"second"}, second, {id:"r2",turnId:"two",role:"assistant",content:"done",turnFinished:true},
    ];
    const attachment = () => <span />;
    await view.render(messages, false, attachment, () => new Set());
    expect(view.host.querySelector('[data-agent-message-id="r1"] [aria-label="本轮变更"]')?.textContent).toContain("a.tex");
    expect(view.host.querySelector('[data-agent-message-id="r1"] [aria-label="本轮变更"]')?.textContent).toContain("b.tex");
    await view.render(messages, false, attachment, turn => new Set(turn === "one" ? ["source://writings/project/a.tex"] : []));
    expect(view.host.querySelector('[data-agent-message-id="r1"] [aria-label="本轮变更"]')?.textContent).not.toContain("a.tex");
    expect(view.host.querySelector('[data-agent-message-id="r1"] [aria-label="本轮变更"]')?.textContent).toContain("b.tex");
    expect(view.host.querySelector('[data-agent-message-id="r2"] [aria-label="本轮变更"]')?.textContent).toContain("a.tex");
    await view.render(messages, false, attachment, () => new Set());
    expect(view.host.querySelector('[data-agent-message-id="r1"] [aria-label="本轮变更"]')?.textContent).toContain("a.tex");
  } finally { await view.close(); }
});
it("withholds a streaming or unfinished summary and retains receipts for a canceled turn without final text", async () => {
  const view = await setup();
  try {
    const messages: WritingAgentMessage[] = [{ id: "user", turnId: "one", role: "user", content: "修改" }, result("one", "tool")];
    await view.render(messages.map(message => ({ ...message, turnFinished: false })), true);
    expect(view.host.querySelector('[aria-label="本轮变更"]')).toBeNull();
    await view.render([{ ...messages[0]!, turnFinished: true }, { ...messages[1]!, turnFinished: true, streaming: true }]);
    expect(view.host.querySelector('[aria-label="本轮变更"]')).toBeNull();
    await view.render(messages.map(message => ({ ...message, turnFinished: true, turnOutcome: "canceled" })));
    const card = view.host.querySelector('[aria-label="本轮变更"]');
    expect(card).not.toBeNull();
    expect(view.host.querySelector(".writing-agent-turn")?.nextElementSibling).toBe(card);
  } finally { await view.close(); }
});
it("keeps patch results attached to their own earlier turn when a later message arrives", async () => {
  const view = await setup();
  try {
    await view.render([{ id: "user-one", turnId: "one", role: "user", content: "修改" }, result("one", "result"),
      { id: "reply-one", turnId: "one", role: "assistant", content: "第一轮", turnFinished: true },
      { id: "user-two", turnId: "two", role: "user", content: "读取" }, { id: "reply-two", turnId: "two", role: "assistant", content: "第二轮", turnFinished: true }]);
    expect(view.host.querySelector('[data-agent-message-id="reply-one"] [aria-label="本轮变更"]')).not.toBeNull();
    expect(view.host.querySelector('[data-agent-message-id="reply-two"] [aria-label="本轮变更"]')).toBeNull();
  } finally { await view.close(); }
});
