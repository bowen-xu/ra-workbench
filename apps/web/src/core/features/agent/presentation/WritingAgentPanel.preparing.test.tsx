/** @vitest-environment happy-dom */

import {act} from "react";
import {createRoot, type Root} from "react-dom/client";
import {EditorView} from "@codemirror/view";
import type {AgentInputDraft} from "@research-assistant/core/agent";
import {afterEach, beforeEach, describe, expect, it, vi} from "vitest";
import {composerEditorDraft} from "../agentComposerEditor";
import {WritingAgentPanel} from "./WritingAgentPanel";

describe("composer request preparation", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
    const storage = new Map<string, string>();
    Object.defineProperty(window, "localStorage", {configurable: true, value: {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => storage.set(key, value),
      removeItem: (key: string) => storage.delete(key), clear: () => storage.clear(),
    }});
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it.each([false, true])("preserves the mounted editor and draft across checking and recovery (rich=%s)", async rich => {
    const draft: AgentInputDraft = rich ? {text: "整理 @笔记", bindings: [{startUtf16: 3, endUtf16: 6,
      input: {type: "mention", name: "笔记", target: {kind: "file", sourceId: "local", resourceId: "note-1", revision: "1"}}}]} : {
      text: "第一行\n保留这条待发送草稿", bindings: [],
    };
    const changed = vi.fn(), submitted = vi.fn(), steered = vi.fn();
    const access = {draft: () => draft, set: changed};
    const render = (isSubmitting: boolean) => act(async () => root.render(<WritingAgentPanel open
      draft={draft.text} draftResources={access} isSubmitting={isSubmitting} onSubmit={submitted} onSteer={steered} />));
    await render(false);
    const input = container.querySelector<HTMLElement>(rich ? ".cm-editor" : "textarea")!;
    const editor = rich ? EditorView.findFromDOM(input)! : null;
    const originalDraft = editor ? composerEditorDraft(editor.state) : null;
    changed.mockClear();
    await render(true);
    expect(container.querySelector(rich ? ".cm-editor" : "textarea")).toBe(input);
    const status = container.querySelector('[role="status"][aria-label="输入框准备状态"]')!;
    expect(status.textContent).toBe("正在核对请求……");
    expect(status.closest(".agent-composer-input-state")?.classList.contains("is-preparing")).toBe(true);
    expect(status.closest(".writing-agent-messages")).toBeNull();
    expect(container.querySelector('[data-action="send"]')?.getAttribute("aria-busy")).toBe("true");
    if (editor) {
      expect(editor.contentDOM.getAttribute("contenteditable")).toBe("false");
      expect(composerEditorDraft(editor.state)).toEqual(originalDraft);
    } else expect((input as HTMLTextAreaElement).disabled).toBe(true);
    await act(async () => {
      container.querySelector("form")?.dispatchEvent(new Event("submit", {bubbles: true, cancelable: true}));
      input.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", ctrlKey: true, bubbles: true, cancelable: true}));
    });
    expect(submitted).not.toHaveBeenCalled();
    expect(steered).not.toHaveBeenCalled();
    expect(changed).not.toHaveBeenCalled();
    await render(false);
    expect(container.querySelector(rich ? ".cm-editor" : "textarea")).toBe(input);
    expect(container.querySelector(".agent-composer-preparing-state")).toBeNull();
    if (editor) {
      expect(EditorView.findFromDOM(input)).toBe(editor);
      expect(composerEditorDraft(editor.state)).toEqual(originalDraft);
      expect(editor.contentDOM.getAttribute("contenteditable")).toBe("true");
    } else {
      expect((input as HTMLTextAreaElement).value).toBe(draft.text);
      expect((input as HTMLTextAreaElement).disabled).toBe(false);
    }
    expect(changed).not.toHaveBeenCalled();
  });

  it("shows queue progress in the composer and keeps recovery actionable without losing the draft", async () => {
    const recover = vi.fn(), submitted = vi.fn();
    const render = (queuePreparation?: {status: "sending" | "recovery"; onRecover?: () => void; recoveryDisabled?: boolean}) => act(async () => root.render(
      <WritingAgentPanel open draft="保留的输入" onSubmit={submitted} queuePreparation={queuePreparation} />));
    await render();
    const editor = container.querySelector<HTMLTextAreaElement>("textarea")!;
    await render({status: "sending"});
    expect(container.querySelector("textarea")).toBe(editor);
    expect(editor.disabled).toBe(true);
    expect(container.querySelector('[role="status"][aria-label="输入框准备状态"]')?.textContent).toBe("发送中…");
    expect(container.querySelector('[data-action="send"]')?.hasAttribute("disabled")).toBe(true);
    await act(async () => container.querySelector("form")!.dispatchEvent(new Event("submit", {bubbles: true, cancelable: true})));
    expect(submitted).not.toHaveBeenCalled();

    await render({status: "recovery", onRecover: recover});
    expect(container.querySelector('[role="status"][aria-label="输入框准备状态"]')?.textContent).toBe("消息发送状态待确认");
    const button = [...container.querySelectorAll("button")].find(item => item.textContent === "处理消息")!;
    expect(button.closest(".agent-composer-input-state")).not.toBeNull();
    expect(button.disabled).toBe(false);
    await act(async () => button.click());
    expect(recover).toHaveBeenCalledOnce();
    await render({status: "recovery", onRecover: recover, recoveryDisabled: true});
    expect([...container.querySelectorAll("button")].find(item => item.textContent === "处理消息")?.disabled).toBe(true);
    await render();
    expect(container.querySelector("textarea")).toBe(editor);
    expect(editor.disabled).toBe(false);
    expect(editor.value).toBe("保留的输入");
  });

  it("offers a stop control during a cancellable send without unlocking the editor", async () => {
    const cancel = vi.fn();
    const render = (isSubmitting: boolean, cancelSendingPending = false) => act(async () => root.render(<WritingAgentPanel open draft="消息"
      isSubmitting={isSubmitting} onCancelSending={cancel} cancelSendingPending={cancelSendingPending} />));
    await render(false);
    expect(container.querySelector('[aria-label="停止发送"]')).toBeNull();
    await render(true);
    const button = container.querySelector<HTMLButtonElement>('[aria-label="停止发送"]')!;
    expect(button.disabled).toBe(false);
    expect(button.classList.contains("writing-agent-stop")).toBe(true);
    expect(container.querySelector<HTMLTextAreaElement>("textarea")!.disabled).toBe(true);
    await act(async () => button.click());
    expect(cancel).toHaveBeenCalledOnce();
    expect(container.querySelector<HTMLTextAreaElement>("textarea")!.disabled).toBe(true);
    await render(true, true);
    expect(container.querySelector<HTMLButtonElement>('[aria-label="停止发送"]')!.disabled).toBe(true);
    expect(container.querySelector('[aria-label="停止发送"]')?.getAttribute("aria-busy")).toBe("true");
    expect(container.querySelector('[aria-label="输入框准备状态"]')?.textContent).toBe("正在停止发送……");
    await render(false, true);
    expect(container.querySelector<HTMLTextAreaElement>("textarea")!.disabled).toBe(true);
    expect(container.querySelector<HTMLButtonElement>('[aria-label="停止发送"]')!.disabled).toBe(true);
  });

  it("does not attach a rejected send's focus intent to a later unrelated submission", async () => {
    const render = (isSubmitting: boolean) => act(async () => root.render(<WritingAgentPanel open draft="消息"
      onSubmit={() => false} isSubmitting={isSubmitting} approvalConversationId="conversation" />));
    await render(false);
    const input = container.querySelector<HTMLTextAreaElement>("textarea")!; input.focus();
    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true})));
    const focus = vi.spyOn(input, "focus");
    await render(true);
    await render(false);
    expect(focus).not.toHaveBeenCalled();
  });

  it.each(["keyboard", "button"] as const)("returns focus after a %s send when the user has not moved it", async method => {
    const submit = vi.fn();
    const render = (isSubmitting: boolean) => act(async () => root.render(<WritingAgentPanel open draft="消息"
      onSubmit={submit} isSubmitting={isSubmitting} approvalConversationId="conversation" />));
    await render(false);
    const input = container.querySelector<HTMLTextAreaElement>("textarea")!;
    if (method === "keyboard") input.focus();
    else container.querySelector<HTMLButtonElement>('[data-action="send"]')!.focus();
    await act(async () => {
      if (method === "keyboard") input.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true}));
      else container.querySelector<HTMLButtonElement>('[data-action="send"]')!.click();
    });
    expect(submit).toHaveBeenCalledOnce();
    // The host may enter its sending state on a later event-loop turn.
    await act(async () => new Promise<void>(resolve => window.setTimeout(resolve, 0)));
    await render(true);
    expect(input.disabled).toBe(true);
    await render(false);
    expect(document.activeElement).toBe(input);
  });

  it("does not reclaim focus when the user moves it elsewhere during the send", async () => {
    const elsewhere = document.createElement("button"); document.body.append(elsewhere);
    const render = (isSubmitting: boolean, conversation = "conversation") => act(async () => root.render(<WritingAgentPanel open draft="消息"
      onSubmit={() => undefined} isSubmitting={isSubmitting} approvalConversationId={conversation} />));
    try {
      await render(false);
      const input = container.querySelector<HTMLTextAreaElement>("textarea")!; input.focus();
      await act(async () => container.querySelector("form")!.dispatchEvent(new Event("submit", {bubbles: true, cancelable: true})));
      await render(true);
      await act(async () => {elsewhere.dispatchEvent(new MouseEvent("mousedown", {bubbles: true})); elsewhere.focus();});
      await render(false);
      expect(document.activeElement).toBe(elsewhere);
      input.focus();
      await act(async () => container.querySelector("form")!.dispatchEvent(new Event("submit", {bubbles: true, cancelable: true})));
      await render(true);
      elsewhere.focus();
      await render(false, "another-conversation");
      expect(document.activeElement).toBe(elsewhere);
    } finally {elsewhere.remove();}
  });

  it("respects a focus change before the host enters its sending state", async () => {
    const elsewhere = document.createElement("button"); document.body.append(elsewhere);
    const render = (isSubmitting: boolean) => act(async () => root.render(<WritingAgentPanel open draft="消息"
      onSubmit={() => undefined} isSubmitting={isSubmitting} approvalConversationId="conversation" />));
    try {
      await render(false);
      const input = container.querySelector<HTMLTextAreaElement>("textarea")!; input.focus();
      await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true})));
      await act(async () => elsewhere.focus());
      await render(true);
      await render(false);
      expect(document.activeElement).toBe(elsewhere);
    } finally {elsewhere.remove();}
  });

  it("does not reclaim focus after a click on non-focusable content", async () => {
    const elsewhere = document.createElement("div"); document.body.append(elsewhere);
    const render = (isSubmitting: boolean) => act(async () => root.render(<WritingAgentPanel open draft="消息"
      onSubmit={() => undefined} isSubmitting={isSubmitting} approvalConversationId="conversation" />));
    try {
      await render(false);
      const input = container.querySelector<HTMLTextAreaElement>("textarea")!; input.focus();
      await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true})));
      await render(true);
      const focus = vi.spyOn(input, "focus");
      await act(async () => elsewhere.dispatchEvent(new MouseEvent("mousedown", {bubbles: true})));
      await render(false);
      expect(focus).not.toHaveBeenCalled();
    } finally {elsewhere.remove();}
  });

  it("does not reclaim focus after the panel is hidden or the conversation changes", async () => {
    const render = (isSubmitting: boolean, open: boolean, conversation: string) => act(async () => root.render(<WritingAgentPanel open={open} draft="消息"
      onSubmit={() => undefined} isSubmitting={isSubmitting} approvalConversationId={conversation} />));
    await render(false, true, "one");
    const input = container.querySelector<HTMLTextAreaElement>("textarea")!; input.focus();
    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true})));
    await render(true, true, "one");
    const focus = vi.spyOn(input, "focus");
    await render(true, false, "one");
    await render(false, false, "one");
    expect(focus).not.toHaveBeenCalled();
    await render(false, true, "one"); input.focus();
    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true})));
    await render(true, true, "one");
    focus.mockClear();
    await render(false, true, "two");
    expect(focus).not.toHaveBeenCalled();
  });

  it("restores focus inside the enclosing modal but not over a different modal", async () => {
    const enclosing = document.createElement("div"); enclosing.setAttribute("role", "dialog"); enclosing.setAttribute("aria-modal", "true");
    document.body.append(enclosing); enclosing.append(container);
    const render = (isSubmitting: boolean) => act(async () => root.render(<WritingAgentPanel open draft="消息"
      onSubmit={() => undefined} isSubmitting={isSubmitting} approvalConversationId="conversation" />));
    let other: HTMLDivElement | null = null;
    try {
      await render(false);
      const input = container.querySelector<HTMLTextAreaElement>("textarea")!; input.focus();
      await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true})));
      await render(true);
      const focus = vi.spyOn(input, "focus");
      await render(false);
      expect(focus).toHaveBeenCalledOnce();
      await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true})));
      await render(true);
      other = document.createElement("div"); other.setAttribute("role", "dialog"); other.setAttribute("aria-modal", "true"); document.body.append(other);
      focus.mockClear();
      await render(false);
      expect(focus).not.toHaveBeenCalled();
    } finally {other?.remove(); document.body.append(container); enclosing.remove();}
  });
});
