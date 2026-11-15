/** @vitest-environment happy-dom */
import { act, StrictMode, useRef, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { AgentComposerTextInput, type AgentComposerInputHandle } from "./AgentComposerTextInput";
import { useAgentComposerInput } from "./useAgentComposerInput";
import { createMockAgentResourceGateways } from "../../../app/previews/agent/mockAgentResourceGateways";
import { notify } from "../../services/notifications";
import { EditorView } from "@codemirror/view";
import type { AgentCommandGateway } from "./agentCommandGateway";
import { AgentCompactionActivity } from "./AgentCompactionActivity";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root;
const resources = createMockAgentResourceGateways();
const sent = vi.fn();
function Harness({ gateway }: { readonly gateway?: AgentCommandGateway }) {
  const [text, setText] = useState("");
  const input = useRef<AgentComposerInputHandle>(null);
  const composer = useAgentComposerInput({ text, setText, input, gateways: resources,
    ...(gateway ? { commandGateway: gateway, scope: { kind: "manuscript", resourceId: "manuscript", conversationId: "conversation" } as const } : {}) });
  return <form onSubmit={event => { event.preventDefault(); if (!composer.consumeCommand()) sent(text); }}>
    <AgentCompactionActivity records={composer.commandActivities} />{composer.menu}<AgentComposerTextInput inputRef={input} aria-label="消息" draft={composer.draft}
      onChange={composer.change} onDraftChange={composer.changeDraft} onCaretChange={composer.update}
      onKeyDown={event => { if (!composer.keyDown(event) && !event.nativeEvent.isComposing && event.key === "Enter") sent(text); }} />
    <button type="submit">发送</button><output>{JSON.stringify(composer.userInput())}</output>
  </form>;
}
async function mount(gateway?: AgentCommandGateway) {
  const host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root.render(<StrictMode><Harness {...(gateway ? { gateway } : {})} /></StrictMode>));
  return host;
}
async function type(host: HTMLElement, text: string) {
  const input = host.querySelector("textarea")!;
  await act(async () => { input.focus(); Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, text);
    input.setSelectionRange(text.length, text.length); input.dispatchEvent(new Event("input", { bubbles: true })); });
  await act(async () => new Promise<void>(resolve => requestAnimationFrame(() => resolve())));
  return input;
}
async function key(input: HTMLElement, key: string, extra: KeyboardEventInit = {}) {
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...extra });
  await act(async () => input.dispatchEvent(event));
  await act(async () => new Promise<void>(resolve => requestAnimationFrame(() => resolve())));
  return event;
}
afterEach(async () => { if (root) await act(async () => root.unmount()); document.body.replaceChildren(); sent.mockClear(); vi.restoreAllMocks(); });

it("highlights the first matching command while typing without moving input focus", async () => {
  const host = await mount();
  for (const query of ["/", "/c", "/compa"]) {
    const input = await type(host, query);
    const item = document.body.querySelector('[role="menuitem"]');
    expect(item?.hasAttribute("data-active-item"), item?.outerHTML).toBe(true);
    expect(document.activeElement).toBe(input);
    expect(input.value).toBe(query);
    expect(input.selectionStart).toBe(query.length);
  }
});

function pendingCommandGateway(): AgentCommandGateway {
  return {
    readUsage: vi.fn(async () => ({ inputTokens: 82000, contextWindowTokens: 128000 })),
    compact: vi.fn(() => new Promise(() => {})),
  };
}

it("Tab executes the selected command and immediately clears the draft without loading resources", async () => {
  const list = vi.spyOn(resources.skills, "list");
  const gateway = pendingCommandGateway();
  const host = await mount(gateway), input = await type(host, "/co");
  expect(document.body.querySelector('[role="menu"][aria-label="Agent 命令"]')?.textContent).toContain("/compact");
  expect(document.body.querySelector('[aria-label="选择 Agent 资源"]')).toBeNull();
  expect(list).not.toHaveBeenCalled();
  await key(input, "ArrowDown");
  expect(document.activeElement).toBe(input);
  expect((await key(input, "Tab")).defaultPrevented).toBe(true);
  expect(input.value).toBe("");
  expect(gateway.compact).toHaveBeenCalledOnce();
  expect(host.querySelector('[role="status"]')?.textContent).toBe("正在压缩上下文");
  expect(document.activeElement).toBe(input);
  expect(document.body.querySelector('[aria-label="Agent 命令"]')).toBeNull();
  // Late selection events for the consumed query must not reopen the popup.
  await act(async () => {
    const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!;
    for (const value of ["/co", ""]) {
      set.call(input, value); input.setSelectionRange(value.length, value.length);
      input.dispatchEvent(new Event("select", { bubbles: true }));
    }
  });
  expect(document.body.querySelector('[aria-label="Agent 命令"]')).toBeNull();
  expect(sent).not.toHaveBeenCalled();
  expect(host.querySelector("output")?.textContent).toBe("[]");
  await type(host, "继续输入");
  expect(input.value).toBe("继续输入");
});
it("Escape dismisses without executing, while send still consumes the command", async () => {
  const info = vi.spyOn(notify, "info").mockReturnValue(1);
  const host = await mount(), input = await type(host, "/compact");
  expect((await key(input, "Escape")).defaultPrevented).toBe(true);
  expect(document.body.querySelector('[aria-label="Agent 命令"]')).toBeNull(); expect(info).not.toHaveBeenCalled();
  await act(async () => host.querySelector<HTMLButtonElement>('button[type="submit"]')!.click());
  expect(info).toHaveBeenCalledOnce(); expect(sent).not.toHaveBeenCalled(); expect(input.value).toBe("/compact");
});
it("pointer selection survives transient blur and never creates resource bindings", async () => {
  const info = vi.spyOn(notify, "info").mockReturnValue(1);
  const host = await mount(); await type(host, "/");
  const item = document.body.querySelector<HTMLButtonElement>('[role="menuitem"]')!;
  await act(async () => {
    item.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    document.activeElement?.dispatchEvent(new FocusEvent("focusout", { bubbles: true, relatedTarget: null }));
    item.dispatchEvent(new PointerEvent("pointerup", { bubbles: true })); item.click();
  });
  expect(info).toHaveBeenCalledOnce(); expect(sent).not.toHaveBeenCalled();
  expect(host.querySelector("textarea")?.value).toBe("/"); expect(host.querySelector("output")?.textContent).toBe("[]");
});
it("handles unknown commands and arguments locally and leaves ordinary @ resources intact", async () => {
  const error = vi.spyOn(notify, "error").mockReturnValue(1);
  const host = await mount(); let input = await type(host, "/unknown");
  expect(document.body.textContent).toContain("没有匹配的命令");
  await key(input, "Enter"); expect(error).toHaveBeenLastCalledWith("未知命令：/unknown。");
  input = await type(host, "/compact 参数"); await key(input, "Enter");
  expect(error).toHaveBeenLastCalledWith("/compact 不接受参数，请只输入 /compact。"); expect(sent).not.toHaveBeenCalled();
  await type(host, "@模拟写作技能");
  expect(document.body.querySelector('[aria-label="选择 Agent 资源"]')).not.toBeNull(); expect(document.body.querySelector('[aria-label="Agent 命令"]')).toBeNull();
});
it("composition and Shift+Enter do not dispatch commands", async () => {
  const info = vi.spyOn(notify, "info").mockReturnValue(1);
  const host = await mount(), input = await type(host, "/compact");
  expect((await key(input, "Enter", { isComposing: true })).defaultPrevented).toBe(false);
  expect((await key(input, "Enter", { shiftKey: true })).defaultPrevented).toBe(false);
  expect((await key(input, "Tab", { isComposing: true })).defaultPrevented).toBe(true);
  expect((await key(input, "Tab", { shiftKey: true })).defaultPrevented).toBe(true);
  expect(info).not.toHaveBeenCalled();
});
it("Tab executes and clears a command in the rich editor without reopening its menu", async () => {
  const gateway = pendingCommandGateway();
  const host = await mount(gateway); await type(host, "@模拟写作技能");
  await act(async () => Array.from(document.body.querySelectorAll<HTMLButtonElement>('[aria-label="选择 Agent 资源"] button'))
    .find(button => button.textContent?.includes("模拟写作技能"))!.click());
  const element = host.querySelector<HTMLElement>(".cm-editor")!;
  const editor = EditorView.findFromDOM(element)!;
  await act(async () => editor.dispatch({changes: {from: 0, to: editor.state.doc.length, insert: "/co"}, selection: {anchor: 3}}));
  await key(editor.contentDOM, "Tab");
  // Clearing a submitted rich draft starts a fresh textarea editing session.
  expect(element.isConnected).toBe(false);
  expect(host.querySelector("textarea")?.value).toBe("");
  expect(document.body.querySelector('[aria-label="Agent 命令"]')).toBeNull();
  expect(gateway.compact).toHaveBeenCalledOnce(); expect(sent).not.toHaveBeenCalled();
});
