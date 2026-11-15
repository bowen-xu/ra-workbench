/** @vitest-environment happy-dom */
import { act, useMemo, useRef } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { AgentConversationSummary } from "@research-assistant/core/agent";
import { useConversationListWindow } from "./useConversationListWindow";

it("recycles distant rows and retains the focused row until focus leaves", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const frames = new Map<number, FrameRequestCallback>(); let frame = 0;
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {frames.set(++frame, callback); return frame;});
  vi.stubGlobal("cancelAnimationFrame", (id: number) => frames.delete(id));
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockImplementation(function(this: HTMLElement) {return this.id === "pane" ? 300 : 0;});
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function(this: HTMLElement) {
    if (this.id === "pane") return new DOMRect(0, 0, 400, 300);
    if (this.id === "rows") return new DOMRect(0, -(this.closest<HTMLElement>("#pane")?.scrollTop ?? 0), 400, 6000);
    if (this.dataset.conversationId) return new DOMRect(0, 0, 400, 30);
    return new DOMRect();
  });
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  function Fixture() {
    const pane = useRef<HTMLElement>(null);
    const items = useMemo(() => Array.from({length: 200}, (_, index): AgentConversationSummary => ({id: String(index), title: `会话${index}`, archived: false, durableCount: 0, createdAt: "", updatedAt: ""})), []);
    const windowed = useConversationListWindow(items, pane);
    return <section id="pane" ref={pane}><button>工具栏</button><div id="rows" ref={windowed.body}>{windowed.rows.map(({item, gap}) => <div key={item.id}>
      {gap > 0 && <div style={{height: gap}} />}<div data-conversation-id={item.id}><button>{item.title}</button></div>
    </div>)}<div style={{height: windowed.bottom}} /></div></section>;
  }
  async function flush() {await act(async () => {const pending = [...frames.values()]; frames.clear(); pending.forEach(callback => callback(0));});}
  try {
    await act(async () => root.render(<Fixture />)); await flush();
    expect(host.querySelectorAll("[data-conversation-id]").length).toBeLessThan(25);
    await act(async () => host.querySelector<HTMLButtonElement>('[data-conversation-id="0"] button')!.focus());
    const pane = host.querySelector<HTMLElement>("#pane")!; pane.scrollTop = 1200;
    await act(async () => pane.dispatchEvent(new Event("scroll"))); await flush();
    expect(host.querySelector('[data-conversation-id="0"] button')).toBe(document.activeElement);
    expect(host.querySelector('[data-conversation-id="40"]')).not.toBeNull();
    await act(async () => host.querySelector<HTMLButtonElement>("#pane > button")!.focus()); await flush();
    expect(host.querySelector('[data-conversation-id="0"]')).toBeNull();
    pane.scrollTop = 0; await act(async () => pane.dispatchEvent(new Event("scroll"))); await flush();
    expect(host.querySelector('[data-conversation-id="0"]')).not.toBeNull();
  } finally {await act(async () => root.unmount()); host.remove(); vi.restoreAllMocks(); vi.unstubAllGlobals();}
});
