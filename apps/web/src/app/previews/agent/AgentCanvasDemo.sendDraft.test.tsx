/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { AgentCanvasDemo } from "./AgentCanvasDemo";
import { selectAppStorage } from "../../../core/runtime/appStorage";

(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
it("consumes the first successful send after the pending draft becomes a durable chat", async () => {
  const values = new Map<string, string>();
  selectAppStorage({get length(){return values.size;}, key:index=>[...values.keys()][index]??null,
    getItem:key=>values.get(key)??null, setItem:(key,value)=>{values.set(key,value);}, removeItem:key=>{values.delete(key);}, clear:()=>values.clear()});
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => root.render(<AgentCanvasDemo/>));
    await vi.waitFor(async () => { await act(async () => Promise.resolve()); expect(host.querySelector('[data-action="stop"]')).not.toBeNull(); });
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="新建会话"]')!.click());
    await act(async () => {
      const input = host.querySelector("textarea")!;
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, "hi");
      input.dispatchEvent(new Event("input", {bubbles: true}));
    });
    await act(async () => host.querySelector<HTMLButtonElement>('[data-action="send"]')!.click());
    await vi.waitFor(async () => { await act(async () => Promise.resolve()); expect(host.querySelector(".writing-agent-transcript")?.textContent).toContain("hi"); });
    expect(host.querySelector("textarea")!.value).toBe("");
    expect(host.querySelector(".writing-agent-transcript")?.textContent).toContain("hi");
  } finally { await act(async () => root.unmount()); host.remove(); }
});
