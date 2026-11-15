/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { AgentCanvasDemo } from "./AgentCanvasDemo";
import { enqueueCanvasMessage } from "./AgentCanvasDemo.testSupport";
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT: boolean}).IS_REACT_ACT_ENVIRONMENT = true;

it("edits through the main composer, replaces its draft, restores the old queue slot, then appends new messages", async () => {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const composer = () => host.querySelector<HTMLTextAreaElement>('[aria-label="给 AI 写作助手发送消息"]')!;
  const rows = () => [...host.querySelectorAll<HTMLLIElement>(".agent-input-queue-body li")];
  async function type(value: string) {await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set?.call(composer(), value);
    composer().dispatchEvent(new Event("input", {bubbles: true}));
  });}
  async function settle(assert: () => void) {await vi.waitFor(async () => {await act(async () => Promise.resolve()); assert();}, {timeout: 4000});}
  try {
    await act(async () => root.render(<AgentCanvasDemo />));
    await enqueueCanvasMessage(host,"修改 keywords 部分");
    await enqueueCanvasMessage(host,"按百分比报告进度");
    await settle(() => expect(rows()).toHaveLength(2));
    await type("被替换的草稿");
    await act(async () => rows()[0]?.querySelector<HTMLButtonElement>('[aria-label="第 1 条消息菜单"]')?.click());
    await act(async () => [...document.querySelectorAll<HTMLButtonElement>('[role="menu"] button')].find(button => button.textContent === "编辑消息")?.click());
    await settle(() => {expect(rows()).toHaveLength(1); expect(composer().value).toBe("修改 keywords 部分");});
    expect(host.querySelector('[aria-label="编辑排队消息"]')).toBeNull();
    expect(document.activeElement).toBe(composer());
    await type("修改 keywords 部分并检查");
    await act(async () => host.querySelector<HTMLButtonElement>('[data-action="send"]')?.click());
    await settle(() => {expect(rows().map(row => row.querySelector("p")?.textContent)).toEqual(["修改 keywords 部分并检查", "按百分比报告进度"]); expect(composer().value).toBe("");});
    await type("新的末尾消息");
    await act(async () => host.querySelector<HTMLButtonElement>('[data-action="send"]')?.click());
    await settle(() => expect(rows().map(row => row.querySelector("p")?.textContent)).toEqual(["修改 keywords 部分并检查", "按百分比报告进度", "新的末尾消息"]));
  } finally {await act(async () => root.unmount()); host.remove();}
});

it("returns an edited message to the paused queue after the running turn has stopped", async () => {
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  try {
    await act(async () => root.render(<AgentCanvasDemo />));
    await vi.waitFor(async () => {await act(async () => Promise.resolve()); expect(host.querySelector('[data-action="stop"]')).not.toBeNull();});
    await enqueueCanvasMessage(host,"修改 keywords 部分");
    await enqueueCanvasMessage(host,"按百分比报告进度");
    await act(async () => host.querySelector<HTMLButtonElement>('[data-action="stop"]')?.click());
    await vi.waitFor(async () => {await act(async () => Promise.resolve()); expect(host.querySelector('[data-action="stop"]')).toBeNull();});
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="第 1 条消息菜单"]')?.click());
    await act(async () => [...document.querySelectorAll<HTMLButtonElement>('[role="menu"] button')].find(button => button.textContent === "编辑消息")?.click());
    await vi.waitFor(async () => {await act(async () => Promise.resolve()); expect(host.querySelectorAll('.agent-input-queue-body li')).toHaveLength(1);});
    await act(async () => host.querySelector<HTMLButtonElement>('[data-action="send"]')?.click());
    await vi.waitFor(async () => {await act(async () => Promise.resolve()); expect([...host.querySelectorAll('.agent-input-queue-body li p')].map(node => node.textContent)).toEqual(["修改 keywords 部分", "按百分比报告进度"]);});
    expect(host.querySelector('.writing-agent-transcript')?.textContent).not.toContain("修改 keywords 部分");
    expect(host.querySelector<HTMLTextAreaElement>('textarea')?.value).toBe("");
    expect(host.querySelector('[data-action="stop"]')).toBeNull();
  } finally {await act(async () => root.unmount()); host.remove();}
});
