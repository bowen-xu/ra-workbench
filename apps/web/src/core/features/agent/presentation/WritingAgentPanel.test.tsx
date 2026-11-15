/** @vitest-environment happy-dom */

import { act, StrictMode } from "react";
import { allowsAppTextSelection } from "../../../uiSelectionPolicy";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { WritingAgentPanel } from "./WritingAgentPanel";
import type { AgentInputDraft } from "@research-assistant/core/agent";
import { createWritingDemoModelGateway } from "../../../../app/previews/agent/WritingDemoModelGateway";

async function setTextareaValue(textarea: HTMLTextAreaElement, value: string): Promise<void> {
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
    setter?.call(textarea, value);
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

async function settleMenu(): Promise<void> {
  await act(async () => { await new Promise<void>((resolve) => requestAnimationFrame(() => resolve())); });
}

describe("WritingAgentPanel", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    const storage = new Map<string, string>();
    Object.defineProperty(window, "localStorage", {
      configurable: true,
      value: {
        clear: () => storage.clear(),
        getItem: (key: string) => storage.get(key) ?? null,
        removeItem: (key: string) => storage.delete(key),
        setItem: (key: string, value: string) => storage.set(key, value),
      },
    });
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: vi.fn(async () => undefined) },
    });
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it("omits right-edge resize controls when docked right and restores them when centered", async () => {
    window.localStorage.setItem("research-assistant.writing-agent.panel-dock", "right");
    await act(async () => root.render(<WritingAgentPanel open />));
    const panel = container.querySelector<HTMLElement>(".writing-agent-panel")!;
    expect(panel.dataset.dock).toBe("right");
    expect(panel.querySelector(".writing-agent-width-handle.right")).toBeNull();
    expect(panel.querySelector(".writing-agent-corner-handle.right")).toBeNull();
    expect(panel.querySelector(".writing-agent-width-handle.left")).not.toBeNull();
    expect(panel.querySelector(".writing-agent-resize")).not.toBeNull();
    await act(async () => panel.querySelector(".writing-agent-dock-handle")?.dispatchEvent(new KeyboardEvent("keydown", {bubbles: true, key: "ArrowLeft"})));
    expect(panel.dataset.dock).toBe("center");
    expect(panel.querySelector(".writing-agent-width-handle.right")).not.toBeNull();
    expect(panel.querySelector(".writing-agent-corner-handle.right")).not.toBeNull();
  });

  it("counts annotations and file attachments while hidden and updates when only references change",async()=>{
    let draft:AgentInputDraft={text:"@附件",bindings:[{startUtf16:0,endUtf16:3,input:{type:"mention",name:"附件",target:{kind:"file",sourceId:"local",resourceId:"attachment",revision:"1"}}}],
      evidence:[{type:"annotation",source:"source://notes/note:#block{b}",title:"原文",text:"quote",comment:""}]};
    const access={draft:()=>draft,set:(next:AgentInputDraft)=>{draft=next;}};
    await act(async()=>root.render(<WritingAgentPanel open={false} draft={draft.text} draftResources={access}/>));
    expect(container.querySelector('.writing-agent-trigger .agent-reference-count-badge')?.textContent).toBe("2");
    draft={...draft,evidence:[]};
    await act(async()=>root.render(<WritingAgentPanel open={false} draft={draft.text} draftResources={access}/>));
    expect(container.querySelector('.writing-agent-trigger .agent-reference-count-badge')?.textContent).toBe("1");
    draft={text:"普通文字",bindings:[]};
    await act(async()=>root.render(<WritingAgentPanel open={false} draft={draft.text} draftResources={access}/>));
    expect(container.querySelector('.writing-agent-trigger .agent-reference-count-badge')).toBeNull();
  });

  it("hides on an outside click without cancelling a turn, losing the draft, or stealing outside focus", async () => {
    const outside = document.createElement("button");
    document.body.append(outside);
    const cancel = vi.fn(), changed = vi.fn(), clicked = vi.fn();
    outside.addEventListener("click", clicked);
    try {
      await act(async () => root.render(<StrictMode><WritingAgentPanel isRunning onCancel={cancel} onOpenChange={changed}
        messages={[{ id: "user", role: "user", content: "现有消息" }]} /></StrictMode>));
      await act(async () => container.querySelector<HTMLButtonElement>('.writing-agent-trigger')?.click());
      await settleMenu();
      await setTextareaValue(container.querySelector<HTMLTextAreaElement>("textarea")!, "未发送草稿");
      await act(async () => container.querySelector('article p')?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
      expect(container.querySelector('[role="dialog"]')).not.toBeNull();
      await act(async () => { outside.focus(); outside.dispatchEvent(new FocusEvent("focusout", { bubbles: true, relatedTarget: null })); });
      expect(container.querySelector('[role="dialog"]')).not.toBeNull();
      await act(async () => {
        outside.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
        outside.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
        outside.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
        outside.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
        outside.click();
      });
      expect(container.querySelector<HTMLElement>('.writing-agent-panel')?.hidden).toBe(true);
      expect(changed).toHaveBeenLastCalledWith(false);
      expect(cancel).not.toHaveBeenCalled();
      expect(clicked).toHaveBeenCalledOnce();
      expect(document.activeElement).toBe(outside);
      await act(async () => container.querySelector<HTMLButtonElement>('.writing-agent-trigger')?.click());
      expect(container.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe("未发送草稿");
      expect(container.textContent).toContain("现有消息");
    } finally { outside.remove(); }
  });

  it("keeps the chat open for its conversation drawer, modal children, and a drag that starts inside", async () => {
    const drawer = document.createElement("aside"), outside = document.createElement("button"), modal = document.createElement("dialog");
    drawer.className = "agent-canvas-conversations";
    const drawerButton = document.createElement("button"), modalButton = document.createElement("button");
    const conversationTrigger = document.createElement("button");
    conversationTrigger.setAttribute("data-agent-conversation-trigger", "");
    drawer.append(drawerButton); modal.append(modalButton);
    document.body.append(drawer, outside, modal, conversationTrigger);
    async function click(element: Element): Promise<void> {
      await act(async () => {
        element.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
        element.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
        element.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      });
    }
    try {
      await act(async () => root.render(<StrictMode><WritingAgentPanel /></StrictMode>));
      await act(async () => container.querySelector<HTMLButtonElement>('.writing-agent-trigger')?.click());
      await settleMenu();
      await click(drawerButton);
      expect(container.querySelector('[role="dialog"]')).not.toBeNull();
      await click(conversationTrigger);
      expect(container.querySelector('[role="dialog"]')).not.toBeNull();
      modal.setAttribute("open", "");
      await click(modalButton);
      expect(container.querySelector('[role="dialog"]')).not.toBeNull();
      modal.removeAttribute("open");
      await act(async () => {
        container.querySelector('textarea')?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
        outside.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
        outside.click();
      });
      expect(container.querySelector('[role="dialog"]')).not.toBeNull();
      await click(outside);
      expect(container.querySelector<HTMLElement>('.writing-agent-panel')?.hidden).toBe(true);
    } finally { drawer.remove(); outside.remove(); modal.remove(); conversationTrigger.remove(); }
  });

  it("switches running return-to-bottom dots to the idle arrow and still returns to the bottom", async () => {
    await act(async () => root.render(<WritingAgentPanel open isRunning />));
    const scroller = container.querySelector<HTMLDivElement>(".writing-agent-messages")!;
    Object.defineProperties(scroller, {
      scrollHeight: { configurable: true, value: 1000 },
      clientHeight: { configurable: true, value: 400 },
    });
    scroller.scrollTop = 400;
    await act(async () => scroller.dispatchEvent(new Event("scroll")));
    const button = container.querySelector<HTMLButtonElement>('[aria-label="回到底部"]')!;
    expect(button.closest(".writing-agent-scroll-bottom-anchor")).not.toBeNull();
    expect(scroller.contains(button)).toBe(true);
    expect(button.dataset.running).toBe("true");
    expect(button.querySelectorAll(".writing-agent-scroll-bottom-dots > span")).toHaveLength(3);
    await act(async () => root.render(<WritingAgentPanel open isRunning={false} />));
    expect(button.dataset.running).toBe("false");
    expect(button.querySelector(".writing-agent-scroll-bottom-dots")).toBeNull();
    expect(button.querySelector(".writing-agent-scroll-bottom-arrow")).not.toBeNull();
    await act(async () => button.click());
    expect(scroller.scrollTop).toBe(600);
    expect(container.querySelector<HTMLButtonElement>('[aria-label="回到底部"]')?.disabled).toBe(true);
  });

  it("keeps queue and goal controls outside the independent message form", async () => {
    const submit = vi.fn();
    await act(async () => root.render(<WritingAgentPanel open onSubmit={submit}
      composerAccessory={<div aria-label="任务区"><button type="button">暂停目标</button></div>} />));
    const taskArea = container.querySelector('[aria-label="任务区"]');
    const form = container.querySelector<HTMLFormElement>("form.writing-agent-composer");
    expect(taskArea?.closest("form")).toBeNull();
    expect(taskArea?.parentElement?.parentElement).toBe(form?.parentElement);
    await act(async () => taskArea?.querySelector<HTMLButtonElement>("button")?.click());
    expect(submit).not.toHaveBeenCalled();
    await setTextareaValue(form!.querySelector<HTMLTextAreaElement>("textarea")!, "独立发送");
    await act(async () => form?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    expect(submit).toHaveBeenCalledWith("独立发送", expect.anything());
  });

  it("leaves Escape in the add menu without stopping the active turn or hiding the panel", async () => {
    const onCancel = vi.fn();
    const onOpenChange = vi.fn();
    await act(async () => root.render(<WritingAgentPanel open isRunning onCancel={onCancel} onOpenChange={onOpenChange} />));
    await settleMenu();
    const add = container.querySelector<HTMLButtonElement>('[aria-label="添加上下文和工具"]')!;
    await act(async () => { add.focus(); add.click(); });
    await settleMenu();
    const item = document.body.querySelector<HTMLButtonElement>('[aria-label="目标模式"]')!;
    const key = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Escape" });
    await act(async () => { item.focus(); item.dispatchEvent(key); });
    await settleMenu();
    expect(key.defaultPrevented).toBe(false);
    expect(onCancel).not.toHaveBeenCalled();
    expect(onOpenChange).not.toHaveBeenCalled();
  });

  it("opens from the floating control, focuses the composer, and restores focus after hiding", async () => {
    await act(async () => root.render(<WritingAgentPanel activePath="main.tex" conversationTitle="Paper" />));
    const trigger = container.querySelector<HTMLButtonElement>('[aria-label="打开 AI 写作助手"]');
    expect(trigger).not.toBeNull();
    expect(container.querySelector('[role="dialog"]')).toBeNull();

    await act(async () => trigger?.click());
    const textarea = container.querySelector<HTMLTextAreaElement>("textarea");
    expect(document.activeElement).toBe(textarea);
    expect(container.textContent).toContain("Paper");
    expect(container.querySelector(".writing-agent-header")?.textContent).toBe("Paper");
    expect(container.querySelector('[role="dialog"]')?.getAttribute("aria-label")).toBe("AI 写作助手");
    expect(container.querySelector('[aria-label="Agent 上下文用量"]')).not.toBeNull();
    expect(container.querySelector('[aria-label="选择 AI 写作模型"]')).not.toBeNull();
    expect(container.querySelector('[aria-label*="语音输入"]')).not.toBeNull();
    expect(container.querySelector('[aria-label="发送消息"]')?.getAttribute("data-action")).toBe("send");

    await setTextareaValue(textarea!, "保留这个草稿");
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="隐藏 AI 写作助手"]')?.click());
    const restoredTrigger = container.querySelector<HTMLButtonElement>('[aria-label="打开 AI 写作助手"]');
    expect(document.activeElement).toBe(restoredTrigger);

    await act(async () => restoredTrigger?.click());
    expect(container.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe("保留这个草稿");
  });

  it("submits with Enter, keeps Shift+Enter for a newline, and stays open with Escape", async () => {
    const onSubmit = vi.fn();
    await act(async () => root.render(<WritingAgentPanel activePath="main.tex" onSubmit={onSubmit} />));
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="打开 AI 写作助手"]')?.click());
    const textarea = container.querySelector<HTMLTextAreaElement>("textarea")!;
    await setTextareaValue(textarea, "  修改摘要  ");
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="添加上下文和工具"]')?.click());
    expect(container.querySelector('[aria-label="整个稿件上下文"]')).toBeNull();
    await act(async () => document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", {bubbles: true, cancelable: true, key: "Escape"})));

    await act(async () => textarea.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Enter", shiftKey: true })));
    expect(onSubmit).not.toHaveBeenCalled();
    expect(textarea.value).toBe("  修改摘要  ");

    await act(async () => textarea.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Enter" })));
    expect(onSubmit).toHaveBeenCalledWith("修改摘要", { contextScope: "current-file" });
    expect(textarea.value).toBe("");

    await act(async () => textarea.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Escape" })));
    expect(container.querySelector('[aria-label="AI 写作助手"]')).not.toBeNull();
  });

  it("does not hide when Escape closes the shared model popup", async () => {
    const { gateway } = createWritingDemoModelGateway();
    await act(async () => root.render(<WritingAgentPanel open modelGateway={gateway} />));
    const trigger = container.querySelector<HTMLButtonElement>('[aria-label="选择 AI 写作模型"]')!;
    await act(async () => trigger.click());
    const popup = document.querySelector<HTMLElement>('[aria-label="选择模型"]')!;
    expect(popup).not.toBeNull();
    const key = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Escape" });
    await act(async () => popup.dispatchEvent(key));
    expect(document.querySelector('[aria-label="可用模型"]')).toBeNull();
    expect(container.querySelector('[aria-label="AI 写作助手"]')).not.toBeNull();
  });

  it("renders conversation content and exposes keyboard height adjustment", async () => {
    await act(async () => root.render(<WritingAgentPanel messages={[
      { id: "u1", role: "user", content: "检查这一段", createdAt: "2026-10-09T00:00:00.100Z" },
      { id: "a1", role: "assistant", content: "建议使用 **更明确** 的主题句；保留文本 [P1-T3]。" },
      { id: "tool1", role: "assistant", kind: "tool-call", toolName: "read_source", content: '{"sourceId":"writing:main.tex"}' },
      { id: "result1", role: "assistant", kind: "tool-result", toolName: "read_source", content: '{"text":"\% Version 3.1"}' },
    ]} />));
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="打开 AI 写作助手"]')?.click());

    expect(container.textContent).toContain("检查这一段");
    expect(container.querySelector(".writing-agent-header-title")?.textContent).toBe("");
    expect(container.querySelector(".writing-agent-header strong")).toBeNull();
    expect(container.querySelector(".writing-agent-message.assistant strong")?.textContent).toBe("更明确");
    expect(container.querySelector('.writing-agent-message.assistant [aria-label="复制消息"]')).not.toBeNull();
    expect(container.querySelectorAll('.writing-agent-message-copy')).toHaveLength(2);
    const userMessage = container.querySelector<HTMLElement>('[data-agent-message-id="u1"]');
    expect(allowsAppTextSelection(userMessage?.querySelector('p')?.firstChild ?? null)).toBe(true);
    expect(allowsAppTextSelection(container.querySelector('.writing-agent-message.assistant strong')?.firstChild ?? null)).toBe(true);
    expect(allowsAppTextSelection(container.querySelector('.writing-agent-header-title'))).toBe(false);
    expect(userMessage?.querySelector('.writing-agent-message-bubble')).not.toBeNull();
    expect(userMessage?.querySelector('.writing-agent-message-meta')).not.toBeNull();
    expect(userMessage?.querySelector('.writing-agent-message-actions')?.parentElement).toBe(userMessage?.querySelector('.writing-agent-message-meta'));
    expect(userMessage?.querySelector('time')?.textContent).toMatch(/\b(?:AM|PM)\b/);
    const activityGroup = container.querySelector<HTMLElement>('[data-agent-activity-single]');
    expect(activityGroup).not.toBeNull();
    expect(activityGroup?.closest('.writing-agent-message')).toBeNull();
    expect(activityGroup?.closest('[data-agent-activity-record]')).not.toBeNull();
    expect(activityGroup?.closest('[data-agent-activity-record]')?.querySelector('.writing-agent-message-time')).toBeNull();
    expect(container.querySelectorAll('[data-agent-activity-group]').length).toBe(0);
    expect(container.querySelectorAll('[data-agent-activity-single]').length).toBe(1);
    expect(activityGroup?.hasAttribute("open")).toBe(false);
    expect(container.querySelector('[data-tool-status="success"]')?.textContent).toContain("已读取");
    expect(container.querySelector('[data-tool-status="success"]')?.hasAttribute("open")).toBe(false);
    await act(async () => activityGroup?.querySelector<HTMLElement>("summary")?.click());
    expect(activityGroup?.hasAttribute("open")).toBe(false);
    expect(container.querySelector('[data-tool-status="success"] summary')).toBeNull();
    expect(container.querySelector('[data-tool-status="success"] pre')).toBeNull();

    const separator = container.querySelector<HTMLElement>('[role="separator"]')!;
    expect(separator.getAttribute("aria-valuenow")).toBe(String(Math.floor(window.innerHeight * 0.5)));
    await act(async () => separator.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "ArrowUp" })));
    expect(separator.getAttribute("aria-valuenow")).toBe(String(Math.floor(window.innerHeight * 0.6)));
  });

  it("renders multiple assistant items from one turn as separate messages", async () => {
    await act(async () => root.render(<WritingAgentPanel open messages={[
      { id: "turn/item-commentary", turnId: "turn", role: "assistant", phase: "commentary", content: "我先检查当前目标。" },
      { id: "turn/item-final", turnId: "turn", role: "assistant", phase: "finalAnswer", content: "检查完成，已准备好结果。" },
    ]} />));
    const messages = [...container.querySelectorAll<HTMLElement>("[data-agent-message-id]")];
    expect(messages.map((message) => message.dataset.agentMessageId)).toEqual(["turn/item-commentary", "turn/item-final"]);
    expect(messages.map((message) => message.textContent)).toEqual([
      expect.stringContaining("我先检查当前目标。"),
      expect.stringContaining("检查完成，已准备好结果。"),
    ]);
  });

  it("copies message text and preserves multiline code blocks", async () => {
    const markdown = ["```tex", "\\\\section{Title}", "text", "```"].join("\n");
    await act(async () => root.render(<WritingAgentPanel open messages={[{
      id: "a1", role: "assistant", content: markdown,
    }]} />));
    const copy = container.querySelector<HTMLButtonElement>('[aria-label="复制消息"]');
    await act(async () => copy?.click());
    expect(navigator.clipboard.writeText).toHaveBeenCalledWith(markdown);
    const block = container.querySelector(".writing-agent-markdown pre");
    expect(block).not.toBeNull();
    expect(block?.textContent).toContain("\\section{Title}");
    expect(container.querySelector('[aria-label="复制代码"]')).not.toBeNull();
  });

  it("keeps the transcript pinned when content resizes at the bottom", async () => {
    await act(async () => root.render(<WritingAgentPanel open messages={[{ id: "a1", role: "assistant", content: "第一段" }]} />));
    const transcript = container.querySelector<HTMLDivElement>(".writing-agent-messages")!;
    let scrollHeight = 240;
    Object.defineProperty(transcript, "clientHeight", { configurable: true, value: 100 });
    Object.defineProperty(transcript, "scrollHeight", { configurable: true, get: () => scrollHeight });
    transcript.scrollTop = 140;
    transcript.dispatchEvent(new Event("scroll"));
    scrollHeight = 320;
    await act(async () => root.render(<WritingAgentPanel open messages={[
      { id: "a1", role: "assistant", content: "第一段" },
      { id: "a2", role: "assistant", content: "追加内容" },
    ]} />));
    expect(transcript.scrollTop).toBe(220);

    await act(async () => {
      transcript.dispatchEvent(new WheelEvent("wheel", {deltaY:-1,bubbles:true,cancelable:true}));
      transcript.scrollTop = 80;
      transcript.dispatchEvent(new Event("scroll"));
    });
    scrollHeight = 400;
    await act(async () => root.render(<WritingAgentPanel open messages={[
      { id: "a1", role: "assistant", content: "第一段" },
      { id: "a2", role: "assistant", content: "追加内容" },
      { id: "a3", role: "assistant", content: "用户正在查看的更早内容" },
    ]} />));
    expect(transcript.scrollTop).toBe(80);
  });
  it("pauses bottom following while reading a nested tool output and resumes explicitly",async()=>{
    const messages=[
      {id:"call",role:"assistant" as const,kind:"tool-call" as const,toolName:"shell",toolCallId:"c",content:'{"cmd":"cat output.log"}'},
      {id:"result",role:"assistant" as const,kind:"tool-result" as const,toolName:"shell",toolCallId:"c",content:"recorded output"},
    ];
    await act(async()=>root.render(<WritingAgentPanel open messages={messages}/>));
    const transcript=container.querySelector<HTMLDivElement>(".writing-agent-messages")!,inner=container.querySelector<HTMLDivElement>(".tool-row-content-viewport")!;
    let height=240;
    Object.defineProperties(transcript,{clientHeight:{configurable:true,value:100},scrollHeight:{configurable:true,get:()=>height}});
    Object.defineProperties(inner,{clientHeight:{configurable:true,value:100},scrollHeight:{configurable:true,value:300}});
    transcript.scrollTop=40;
    await act(async()=>transcript.dispatchEvent(new Event("scroll")));
    await act(async()=>inner.dispatchEvent(new WheelEvent("wheel",{deltaY:20,bubbles:true,cancelable:true})));
    await act(async()=>{await new Promise<void>(resolve=>window.setTimeout(resolve,300));});
    height=320;
    await act(async()=>root.render(<WritingAgentPanel open messages={[...messages,{id:"continued",role:"assistant",content:"继续返回内容"}]}/>));
    expect(transcript.scrollTop).toBe(40);
    await act(async()=>container.querySelector<HTMLButtonElement>('[aria-label="回到底部"]')?.click());
    expect(transcript.scrollTop).toBe(220);
    height=400;
    await act(async()=>root.render(<WritingAgentPanel open messages={[...messages,{id:"continued",role:"assistant",content:"继续返回更多内容"}]}/>));
    expect(transcript.scrollTop).toBe(300);
  });
  it("renders one action slot for a whole tool activity group", async () => {
    const actions = vi.fn((messageId: string) => <button type="button">操作 {messageId}</button>);
    await act(async () => root.render(<WritingAgentPanel open messageActions={actions} messages={[
      { id: "call-1", turnId: "turn-1", role: "assistant", kind: "tool-call", toolName: "read_source", content: "{}" },
      { id: "result-1", turnId: "turn-1", role: "assistant", kind: "tool-result", toolName: "read_source", content: "{}" },
      { id: "call-2", turnId: "turn-1", role: "assistant", kind: "tool-call", toolName: "search_sources", content: "{}" },
      { id: "result-2", turnId: "turn-1", role: "assistant", kind: "tool-result", toolName: "search_sources", content: "{}" },
    ]} />));
    expect(container.querySelectorAll("[data-agent-message-actions]")).toHaveLength(1);
    expect(container.querySelectorAll("[data-agent-message-actions] button")).toHaveLength(1);
    expect(actions).toHaveBeenLastCalledWith("result-2");
  });

  it("offers a bottom recovery action after the user scrolls away", async () => {
    await act(async () => root.render(<WritingAgentPanel open messages={[{ id: "a1", role: "assistant", content: "内容" }]} />));
    const transcript = container.querySelector<HTMLElement>(".writing-agent-messages")!;
    Object.defineProperty(transcript, "clientHeight", { configurable: true, value: 100 });
    Object.defineProperty(transcript, "scrollHeight", { configurable: true, value: 300 });
    transcript.scrollTop = 40;
    await act(async () => transcript.dispatchEvent(new Event("scroll")));
    const button = container.querySelector<HTMLButtonElement>(".writing-agent-scroll-bottom");
    expect(button?.getAttribute("aria-label")).toBe("回到底部");
    await act(async () => button?.click());
    expect(transcript.scrollTop).toBe(200);
    expect(container.querySelector<HTMLButtonElement>(".writing-agent-scroll-bottom")?.disabled).toBe(true);
  });

  it("compresses a contiguous tool run into one expandable activity", async () => {
    await act(async () => root.render(<WritingAgentPanel open messages={[
      { id: "call-1", role: "assistant", kind: "tool-call", toolName: "read_source", content: "{}" },
      { id: "result-1", role: "assistant", kind: "tool-result", toolName: "read_source", content: JSON.stringify({ changes: [{ path: "main.tex" }, { path: "sections/intro.tex" }] }) },
      { id: "call-2", role: "assistant", kind: "tool-call", toolName: "search_sources", content: "{}" },
      { id: "result-2", role: "assistant", kind: "tool-result", toolName: "search_sources", content: "{}" },
    ]} />));
    expect(container.querySelectorAll('[data-agent-activity-group]').length).toBe(1);
    expect(container.querySelector('[data-agent-activity-group] .writing-agent-activity-title')?.textContent).toBe("读取文件");
    expect(container.querySelector('[aria-label="调用结果"]')).toBeNull();
    expect(container.textContent).not.toContain("sections/intro.tex");
  });

  it("adjusts the composer width through fixed presets and keeps dense actions icon-only", async () => {
    await act(async () => root.render(<WritingAgentPanel open onQueue={() => undefined} />));
    const panel = container.querySelector<HTMLElement>('[aria-label="AI 写作助手"]')!;
    expect(panel.dataset.widthPreset).toBe("standard");
    const leftHandle = container.querySelector<HTMLElement>('[aria-label="调整 AI 写作助手左边界"]')!;
    expect(leftHandle.getAttribute("aria-valuenow")).toBe(String(Math.floor(window.innerWidth * 0.5)));
    await act(async () => leftHandle.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "ArrowLeft" })));
    expect(panel.dataset.widthPreset).toBe("compact");
    await act(async () => leftHandle.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "ArrowRight" })));
    expect(panel.dataset.widthPreset).toBe("standard");
    expect(container.querySelector('[aria-label="加入待发送队列"]')).toBeNull();
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="添加上下文和工具"]')?.click());
    expect(document.body.querySelector('[aria-label="概念网络检索"]')).not.toBeNull();
  });

  it("shows removable indicators for active goal and retrieval options", async () => {
    await act(async () => root.render(<WritingAgentPanel open />));
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="添加上下文和工具"]')?.click());
    const goal = document.body.querySelector<HTMLButtonElement>('[aria-label="目标模式"]');
    await act(async () => goal?.click());
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="添加上下文和工具"]')?.click());
    await act(async () => document.body.querySelector<HTMLButtonElement>('[aria-label="概念网络检索"]')?.click());
    expect(container.querySelector('[aria-label="清除目标"]')).not.toBeNull();
    expect(container.querySelector('[aria-label="关闭概念网络检索"]')).not.toBeNull();
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="清除目标"]')?.click());
    expect(container.querySelector('[aria-label="清除目标"]')).toBeNull();
  });

  it("persists both five-level dimensions across closing and reopening", async () => {
    await act(async () => root.render(<WritingAgentPanel open />));
    const panel = container.querySelector<HTMLElement>('[aria-label="AI 写作助手"]')!;
    const widthHandle = container.querySelector<HTMLElement>('[aria-label="调整 AI 写作助手右边界"]')!;
    const heightHandle = container.querySelector<HTMLElement>('[aria-label="调整 AI 写作助手高度"]')!;
    await act(async () => widthHandle.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "ArrowRight" })));
    await act(async () => heightHandle.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "ArrowUp" })));
    expect(panel.dataset.widthPreset).toBe("comfortable");
    expect(panel.dataset.heightPreset).toBe("comfortable");
    await act(async () => root.unmount());
    root = createRoot(container);
    await act(async () => root.render(<WritingAgentPanel open />));
    const reopened = container.querySelector<HTMLElement>('[aria-label="AI 写作助手"]')!;
    expect(reopened.dataset.widthPreset).toBe("comfortable");
    expect(reopened.dataset.heightPreset).toBe("comfortable");
  });

  it("explains rejected tool requests with a safe diagnostic code", async () => {
    await act(async () => root.render(<WritingAgentPanel open messages={[
      { id: "call", role: "assistant", turnId: "turn-error", kind: "tool-call", toolName: "apply_patch", content: "{}" },
      { id: "result", role: "assistant", turnId: "turn-error", kind: "tool-result", toolName: "apply_patch", toolError: true, content: '{"code":"tool_rejected","message":"工具请求被当前执行策略拒绝。","retryable":false}' },
    ]} />));
    expect(container.textContent).toContain("工具请求被当前执行策略拒绝");
    expect(container.textContent).toContain("错误码：tool_rejected");
    expect(container.textContent).not.toContain('"code":"tool_rejected"');
  });

  it("renders a durable model failure as an accessible inline diagnostic", async () => {
    await act(async () => root.render(<WritingAgentPanel open messages={[
      { id: "turn/failure", role: "assistant", content: "Agent 请求失败：模型请求无效", failure: true },
    ]} />));

    const alert = container.querySelector<HTMLElement>('[role="alert"].writing-agent-message.failure');
    expect(alert?.textContent).toContain("Agent 请求失败：模型请求无效");
  });

  it("uses a stop action while a response is running", async () => {
    const onCancel = vi.fn();
    await act(async () => root.render(<WritingAgentPanel isRunning onCancel={onCancel} />));
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="打开 AI 写作助手"]')?.click());
    let stop = container.querySelector<HTMLButtonElement>('[aria-label="停止生成"]');
    expect(stop?.getAttribute("data-action")).toBe("stop");
    await act(async () => stop?.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Escape" })));
    expect(onCancel).not.toHaveBeenCalled();
    expect(container.querySelector('[aria-label="AI 写作助手"]')).not.toBeNull();
    await act(async () => stop?.click());
    expect(onCancel).toHaveBeenCalledOnce();
    expect(stop?.parentElement?.classList.contains("writing-agent-composer-primary")).toBe(true);
    expect(stop?.parentElement?.parentElement?.classList.contains("writing-agent-composer-tools")).toBe(false);
  });

  it("sends a running draft to the queue without an inline steer control", async () => {
    const onQueue = vi.fn();
    const onSteer = vi.fn();
    const onCancel = vi.fn();
    await act(async () => root.render(<WritingAgentPanel open isRunning draft="下一轮说明" onQueue={onQueue} onSteer={onSteer} onCancel={onCancel}
      contextUsage={{ inputTokens: 174000, contextWindowTokens: 258000 }} />));
    const primary = container.querySelector<HTMLButtonElement>('[data-action="send"]');
    expect(primary?.getAttribute("aria-label")).toBe("发送消息");
    expect(primary?.disabled).toBe(false);
    expect(container.querySelector('[aria-label="补充说明"]')).toBeNull();
    expect(container.querySelector<HTMLButtonElement>('[aria-label="Agent 上下文用量"]')?.disabled).toBe(false);
    expect(container.querySelector('.writing-agent-composer-hint')).toBeNull();
    expect(onSteer).not.toHaveBeenCalled();
    await act(async () => primary?.click());
    expect(onQueue).toHaveBeenCalledWith("下一轮说明");
    expect(onCancel).not.toHaveBeenCalled();
    expect(onSteer).not.toHaveBeenCalled();
  });

  it("allows editing and switches between send and stop while running", async () => {
    await act(async () => root.render(<WritingAgentPanel open isRunning onQueue={vi.fn()} onCancel={vi.fn()} />));
    const textarea = container.querySelector<HTMLTextAreaElement>("textarea")!;
    expect(textarea.disabled).toBe(false);
    expect(textarea.readOnly).toBe(false);
    expect(container.querySelector('[data-action="stop"]')).not.toBeNull();
    await setTextareaValue(textarea, "继续检查");
    expect(textarea.value).toBe("继续检查");
    expect(container.querySelector('[data-action="stop"]')).toBeNull();
    expect(container.querySelector<HTMLButtonElement>('[data-action="send"]')?.disabled).toBe(false);
    await setTextareaValue(textarea, " \n ");
    expect(container.querySelector('[data-action="send"]')).toBeNull();
    expect(container.querySelector('[data-action="stop"]')).not.toBeNull();
  });

  it.each([
    { disabledReason: "正在读取对话历史" },
    { historyReady: false },
    { historyError: true },
    { steeringPending: true },
  ])("keeps composer admission states without a floating status message: %j", async (status) => {
    await act(async () => root.render(<WritingAgentPanel open {...status} />));
    expect(container.querySelector(".writing-agent-composer-status")).toBeNull();
    expect(container.querySelector<HTMLButtonElement>('[data-action="send"]')?.disabled).toBe(true);
    expect(container.querySelector(".writing-agent-composer-hint")).toBeNull();
  });

  it("shows the preparing placeholder while history is unavailable and preserves the draft through recovery", async () => {
    const onSubmit = vi.fn(); const onSteer = vi.fn();
    await act(async () => root.render(<WritingAgentPanel open historyReady={false} onSubmit={onSubmit} onSteer={onSteer} />));
    const textarea = container.querySelector<HTMLTextAreaElement>("textarea")!;
    expect(textarea.disabled).toBe(true);
    expect(textarea.placeholder).toBe("准备中……");
    expect(container.querySelector(".agent-composer-preparing-state")?.textContent).toBe("准备中……");
    expect(container.querySelector(".agent-composer-preparing-state")?.getAttribute("role")).toBe("status");
    expect(container.querySelector(".writing-agent-composer-status")).toBeNull();
    await act(async () => root.render(<WritingAgentPanel open historyReady onSubmit={onSubmit} />));
    expect(textarea.disabled).toBe(false);
    await setTextareaValue(textarea, "保留这条草稿");
    await act(async () => root.render(<WritingAgentPanel open historyReady={false} onSubmit={onSubmit} onSteer={onSteer} />));
    expect(textarea.value).toBe("保留这条草稿");
    expect(container.querySelector(".agent-composer-preparing-state")).toBeNull();
    await act(async () => container.querySelector("form")?.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    await act(async () => textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true, cancelable: true })));
    expect(onSubmit).not.toHaveBeenCalled();
    expect(onSteer).not.toHaveBeenCalled();
    await act(async () => root.render(<WritingAgentPanel open historyReady onSubmit={onSubmit} />));
    expect(textarea.disabled).toBe(false);
    expect(textarea.placeholder).toBe("有什么可以帮您");
    expect(textarea.value).toBe("保留这条草稿");
    expect(container.querySelector(".writing-agent-composer-status")).toBeNull();
    expect(container.querySelector(".agent-composer-preparing-state")).toBeNull();
  });

  it("keeps the active turn stoppable and the next draft editable during a history reread", async () => {
    const onCancel = vi.fn();
    await act(async () => root.render(<WritingAgentPanel open isRunning historyReady={false} onCancel={onCancel} />));
    expect(container.querySelector<HTMLTextAreaElement>("textarea")?.disabled).toBe(false);
    await act(async () => container.querySelector<HTMLButtonElement>('[data-action="stop"]')?.click());
    expect(onCancel).toHaveBeenCalledOnce();
  });

  it("allows configuring the next message through the add menu while running", async () => {
    const options = vi.fn(() => null);
    await act(async () => root.render(<WritingAgentPanel open isRunning activePath="main.tex" composerAccessory={options} />));
    await settleMenu();
    const add = container.querySelector<HTMLButtonElement>('[aria-label="添加上下文和工具"]')!;
    expect(add.disabled).toBe(false);
    await act(async () => { add.focus(); add.click(); });
    await settleMenu();
    expect(add.getAttribute("aria-expanded")).toBe("true");
    for (const label of ["目标模式", "概念网络检索"]) {
      const control = document.body.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
      expect(control.disabled).toBe(false);
      await act(async () => control.click());
      await settleMenu();
      expect(add.getAttribute("aria-expanded")).toBe("false");
      await act(async () => { add.focus(); add.click(); });
      await settleMenu();
    }
    expect(options).toHaveBeenLastCalledWith({ contextScope: "current-file", goalMode: true, includeConceptRetrieval: true });
  });

  it.each([
    { queueDisabled: true },
    { steeringPending: true },
    { disabledReason: "对话只读" },
  ])("keeps the send icon disabled when delivery is blocked: %j", async (blocked) => {
    const onQueue = vi.fn();
    await act(async () => root.render(<WritingAgentPanel open isRunning draft="下一条" onQueue={onQueue} {...blocked} />));
    expect(container.querySelector('[data-action="stop"]')).toBeNull();
    const send = container.querySelector<HTMLButtonElement>('[data-action="send"]')!;
    expect(send.disabled).toBe(true);
    await act(async () => send.click());
    await act(async () => container.querySelector("textarea")?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
    expect(onQueue).not.toHaveBeenCalled();
  });

  it("keeps thinking text free of elapsed seconds after remount and while waiting", async () => {
    vi.useFakeTimers();
    try {
      const startedAt = Date.now() - 4_500;
      await act(async () => root.render(<WritingAgentPanel open isRunning thinkingStartedAtMs={startedAt} />));
      const activity = container.querySelector<HTMLElement>('[data-agent-activity="thinking"]');
      expect(activity?.textContent).toBe("思考中");
      await act(async () => { vi.advanceTimersByTime(1_500); });
      expect(activity?.textContent).toBe("思考中");
    } finally {
      vi.useRealTimers();
    }
  });

  it("shows a compact thinking activity while a response is running", async () => {
    await act(async () => root.render(<WritingAgentPanel open isRunning />));
    const activity = container.querySelector<HTMLElement>('[data-agent-activity="thinking"]');
    const composer = container.querySelector<HTMLElement>(".writing-agent-composer");
    expect(activity).not.toBeNull();
    expect(activity?.textContent).toBe("思考中");
    expect(composer).not.toBeNull();
    expect(activity!.compareDocumentPosition(composer!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("keeps thinking at the transcript tail and hides message actions until it completes", async () => {
    await act(async () => root.render(<WritingAgentPanel open isRunning messages={[{
      id: "a1", role: "assistant", content: "正在检查稿件。", streaming: true,
    }]} />));
    const message = container.querySelector<HTMLElement>('[data-agent-message-id="a1"]');
    expect(message?.querySelector('[data-agent-activity="thinking"]')).toBeNull();
    expect(container.querySelector('[data-agent-activity="thinking"]')!.compareDocumentPosition(message!) & Node.DOCUMENT_POSITION_PRECEDING).toBeTruthy();
    expect(message?.querySelector(".writing-agent-message-meta")).toBeNull();
    expect(container.querySelectorAll('[data-agent-activity="thinking"]')).toHaveLength(1);

    await act(async () => root.render(<WritingAgentPanel open messages={[{
      id: "a1", role: "assistant", content: "稿件检查完成。", streaming: false,
    }]} />));
    const completed = container.querySelector<HTMLElement>('[data-agent-message-id="a1"]');
    expect(completed?.querySelector(".writing-agent-message-meta")).not.toBeNull();
    expect(completed?.querySelector('[data-agent-activity="thinking"]')).toBeNull();
  });

  it("opens a child task conversation from the task summary", async () => {
    const onOpenChildTask = vi.fn();
    await act(async () => root.render(<WritingAgentPanel open onOpenChildTask={onOpenChildTask} childTasks={[{
      childConversationId: "11111111-1111-4111-8111-111111111111",
      parentConversationId: "22222222-2222-4222-8222-222222222222",
      parentContextSnapshotId: "33333333-3333-4333-8333-333333333333",
      childContextSnapshotId: "44444444-4444-4444-8444-444444444444",
      depth: 1,
      modelCalls: 1,
      toolCalls: 1,
      status: "running",
      schemaVersion: 1,
      createdAt: "2026-01-01T00:00:00.000Z",
      finishedAt: null,
    }]} />));
    const button = container.querySelector<HTMLButtonElement>('.writing-agent-child-task');
    expect(button).not.toBeNull();
    await act(async () => button?.click());
    expect(onOpenChildTask).toHaveBeenCalledWith("11111111-1111-4111-8111-111111111111");
  });

  it.each([false, true])("leaves Escape to an open modal without hiding or cancelling (running=%s)", async (isRunning) => {
    const onCancel = vi.fn(); const onOpenChange = vi.fn();
    await act(async () => root.render(<WritingAgentPanel open isRunning={isRunning} onCancel={onCancel}
      onOpenChange={onOpenChange} historyHeader={<dialog open><button>Modal action</button></dialog>} />));
    const modal = container.querySelector("dialog")!;
    const key = new KeyboardEvent("keydown", {bubbles:true,cancelable:true,key:"Escape"});
    await act(async () => modal.querySelector("button")?.dispatchEvent(key));
    expect(key.defaultPrevented).toBe(false); expect(onCancel).not.toHaveBeenCalled(); expect(onOpenChange).not.toHaveBeenCalled();
    modal.removeAttribute("open");
    await act(async () => container.querySelector("textarea")?.dispatchEvent(new KeyboardEvent("keydown", {bubbles:true,cancelable:true,key:"Escape"})));
    expect(onCancel).not.toHaveBeenCalled(); expect(onOpenChange).not.toHaveBeenCalled();
  });

  it("queues a second message with Enter and steers only through the explicit action", async () => {
    const onSteer = vi.fn(); const onSubmit = vi.fn(); const onCancel = vi.fn(); const onQueue = vi.fn();
    await act(async () => root.render(<WritingAgentPanel open isRunning draft="clarify" onSteer={onSteer} onSubmit={onSubmit} onQueue={onQueue} onCancel={onCancel} />));
    const textarea = container.querySelector("textarea")!;
    await act(async () => textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", isComposing: true, bubbles: true })));
    expect(onSteer).not.toHaveBeenCalled();
    await act(async () => textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", shiftKey: true, bubbles: true })));
    expect(onSteer).not.toHaveBeenCalled();
    await act(async () => textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
    expect(onSteer).not.toHaveBeenCalled();
    expect(onQueue).toHaveBeenCalledWith("clarify");
    await act(async () => textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true, cancelable: true })));
    expect(onQueue).toHaveBeenCalledExactlyOnceWith("clarify");
    expect(container.querySelector('[aria-label="补充说明"]')).toBeNull();
    expect(container.querySelector('.writing-agent-composer-hint')).toBeNull();
    await act(async () => textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true, cancelable: true })));
    expect(onSteer).toHaveBeenCalledOnce();
    expect(onSubmit).not.toHaveBeenCalled();
    expect(textarea.value).toBe("clarify");
    expect(container.querySelector('[aria-label="发送消息"]')?.getAttribute("data-action")).toBe("send");
  });

  it("keeps Enter queued when a configured queue is temporarily unavailable", async () => {
    const onSteer = vi.fn(); const onQueue = vi.fn();
    await act(async () => root.render(<WritingAgentPanel open isRunning draft="Next question"
      onSteer={onSteer} onQueue={onQueue} queueDisabled />));
    expect(container.querySelector<HTMLButtonElement>('[aria-label="发送消息"]')?.disabled).toBe(true);
    await act(async () => container.querySelector("textarea")?.dispatchEvent(new KeyboardEvent("keydown", {
      key: "Enter", bubbles: true, cancelable: true,
    })));
    expect(onQueue).not.toHaveBeenCalled();
    expect(onSteer).not.toHaveBeenCalled();
    await act(async () => container.querySelector("textarea")?.dispatchEvent(new KeyboardEvent("keydown", {
      key: "Enter", ctrlKey: true, bubbles: true, cancelable: true,
    })));
    expect(onSteer).toHaveBeenCalledExactlyOnceWith("Next question");
  });

  it("uses steer for Enter when no queue gateway exists", async () => {
    const onSteer = vi.fn();
    await act(async () => root.render(<WritingAgentPanel open isRunning draft="Next question" onSteer={onSteer} />));
    await act(async () => container.querySelector("textarea")?.dispatchEvent(new KeyboardEvent("keydown", {
      key: "Enter", bubbles: true, cancelable: true,
    })));
    expect(onSteer).toHaveBeenCalledExactlyOnceWith("Next question");
  });

  it("uses Cmd/Ctrl+Enter for direct steering without queueing while idle", async () => {
    const onSteer = vi.fn(); const onSubmit = vi.fn(); const onQueue = vi.fn();
    await act(async () => root.render(<WritingAgentPanel open isRunning={false} draft="Start directly"
      onSteer={onSteer} onSubmit={onSubmit} onQueue={onQueue} />));
    await act(async () => container.querySelector("textarea")?.dispatchEvent(new KeyboardEvent("keydown", {
      key: "Enter", ctrlKey: true, bubbles: true, cancelable: true,
    })));
    expect(onSteer).toHaveBeenCalledWith("Start directly");
    expect(onQueue).not.toHaveBeenCalled();
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("defaults to the current file when it becomes available after mounting", async () => {
    const onSubmit = vi.fn();
    await act(async () => root.render(<WritingAgentPanel onSubmit={onSubmit} />));
    await act(async () => root.render(<WritingAgentPanel activePath="main.tex" onSubmit={onSubmit} />));
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="打开 AI 写作助手"]')?.click());
    const textarea = container.querySelector<HTMLTextAreaElement>("textarea")!;
    await setTextareaValue(textarea, "检查当前文件");
    await act(async () => textarea.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, key: "Enter" })));
    expect(onSubmit).toHaveBeenCalledWith("检查当前文件", { contextScope: "current-file" });
  });

  it("supports a controlled open state for the conversation list", async () => {
    const onOpenChange = vi.fn();
    await act(async () => root.render(<WritingAgentPanel open onOpenChange={onOpenChange} />));
    expect(container.querySelector('[aria-label="AI 写作助手"]')).not.toBeNull();
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="隐藏 AI 写作助手"]')?.click());
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
});
