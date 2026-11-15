/** @vitest-environment happy-dom */
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { insertAgentReference, type AgentInputDraft } from "@research-assistant/core/agent";
import { WritingAgentPanel } from "./WritingAgentPanel";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

it("adds selected message text to the shared annotation group without changing the draft or existing resources", async () => {
  let draft: AgentInputDraft = { ...insertAgentReference({ text: "已有 ", bindings: [] }, {
    type: "mention", name: "工具", target: { kind: "tool", serverId: "server", environmentId: "test", revision: "v1" },
  }), evidence: [{ type: "annotation", source: "source://note", title: "证据", text: "原文", comment: "" }] };
  const original = draft;
  const onOpenChange = vi.fn();
  const access = { draft: () => draft, set: (next: AgentInputDraft) => { draft = next; } };
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  function Fixture({ sending = false }: { sending?: boolean }) {
    const [text, setText] = useState(original.text);
    return <WritingAgentPanel open onOpenChange={onOpenChange} isSubmitting={sending} draft={text} onDraftChange={setText} draftResources={access}
      messages={[{ id: "assistant", role: "assistant", content: "第一行\n第二行" }]} />;
  }
  async function select() {
    const bubble = host.querySelector("[data-agent-message-id='assistant'] .writing-agent-markdown")!;
    const range = document.createRange(); range.selectNodeContents(bubble);
    range.getBoundingClientRect = () => new DOMRect(30, 80, 100, 20);
    await act(async () => { window.getSelection()!.removeAllRanges(); window.getSelection()!.addRange(range); document.dispatchEvent(new Event("selectionchange")); });
  }
  try {
    await act(async () => root.render(<Fixture />));
    await select();
    await act(async () => {
      const button = document.querySelector<HTMLButtonElement>(".agent-message-selection-action button")!;
      button.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      button.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
      button.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
      button.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
      button.click();
    });
    expect(draft.evidence).toEqual(original.evidence);
    const comment = document.querySelector<HTMLTextAreaElement>('textarea[aria-label="批注 对话选段"]')!;
    expect(comment).not.toBeNull();
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(comment, "解释这句话");
      comment.dispatchEvent(new Event("input", {bubbles: true}));
    });
    await act(async () => comment.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true})));
    expect(draft.text).toBe(original.text);
    expect(draft.bindings).toEqual(original.bindings);
    expect(draft.evidence).toEqual([...original.evidence!, {type: "annotation", title: "对话选段", text: "第一行\n第二行", comment: "解释这句话"}]);
    expect(host.querySelector(".agent-evidence-disclosure")?.textContent).toContain("2 条批注");
    expect(host.querySelector(".writing-agent-panel")?.hasAttribute("hidden")).toBe(false);
    expect(onOpenChange).not.toHaveBeenCalledWith(false);
    await act(async () => root.render(<Fixture sending />));
    await select();
    expect(document.querySelector(".agent-message-selection-action")).toBeNull();
  } finally { await act(async () => root.unmount()); window.getSelection()?.removeAllRanges(); host.remove(); }
});
