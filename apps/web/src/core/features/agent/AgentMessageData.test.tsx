/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { DeveloperModeProvider, DeveloperModeToggle } from "../settings/DeveloperMode";
import { AgentMessageData, type ReadMessageData } from "./AgentMessageData";
import type { WritingAgentMessage } from "./presentation/WritingAgentMessage";
import { AgentCompactionActivity, type AgentCompactionRecord } from "./AgentCompactionActivity";
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root;
const message: WritingAgentMessage = { id: "turn/item", role: "user", content: "display suffix", rawData: { text: "original🧪" } };
beforeEach(() => {
  const values = new Map<string, string>();
  Object.defineProperty(window, "localStorage", { configurable: true, value: { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => values.set(key, value), clear: () => values.clear(), get length() { return values.size; } } });
  vi.spyOn(HTMLDialogElement.prototype, "showModal").mockImplementation(function (this: HTMLDialogElement) { this.setAttribute("open", ""); });
  vi.spyOn(HTMLDialogElement.prototype, "close").mockImplementation(function (this: HTMLDialogElement) { this.removeAttribute("open"); });
  const host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root?.unmount()); document.body.replaceChildren(); window.localStorage.clear(); vi.restoreAllMocks(); });
async function mount(read: ReadMessageData = vi.fn(async () => ({ frozenSubmission: { input: { text: "frozen" } } }))) {
  await act(async () => root.render(<DeveloperModeProvider><DeveloperModeToggle /><AgentMessageData messages={[message]} read={read} /></DeveloperModeProvider>));
  return read;
}
async function click(label: string) {
  const button = [...document.querySelectorAll<HTMLButtonElement>("button")].find(element => (element.getAttribute("aria-label") ?? element.textContent) === label)!;
  await act(async () => button.click());
}
it("defaults off, persists only the flag, and lazily displays original records", async () => {
  const read = await mount();
  expect(document.querySelector(".agent-message-data-trigger")).toBeNull(); expect(read).not.toHaveBeenCalled();
  await act(async () => document.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click());
  expect(window.localStorage.getItem("research-assistant.developer-mode")).toBe("true");
  expect(read).not.toHaveBeenCalled();
  expect(document.querySelector(".agent-message-data-trigger")!.textContent).toBe("");
  expect(document.querySelector(".agent-message-data-trigger svg")).not.toBeNull();
  await click("查看原始数据");
  const data = JSON.parse(document.querySelector("dialog pre")!.textContent!);
  expect(data.message).toEqual(message.rawData); expect(data.record.frozenSubmission.input.text).toBe("frozen");
  expect(window.localStorage.length).toBe(1);
  await click("关闭"); expect(document.querySelector("dialog")).toBeNull();
});
it("restores preference and retries failed reads without showing stale data", async () => {
  window.localStorage.setItem("research-assistant.developer-mode", "true");
  const read = vi.fn().mockRejectedValueOnce(new Error("unavailable")).mockResolvedValueOnce({ events: ["record"] });
  await mount(read); await click("查看原始数据");
  expect(document.querySelector('[role="alert"]')!.textContent).toContain("无法读取");
  expect(document.querySelector("pre")).toBeNull();
  await click("重试读取"); expect(document.querySelector("pre")!.textContent).toContain("record");
  await act(async () => document.querySelector("dialog")!.dispatchEvent(new Event("cancel", { cancelable: true })));
  expect(document.querySelector("dialog")).toBeNull();
});
it("closing while loading discards a late response and disabling closes the dialog", async () => {
  window.localStorage.setItem("research-assistant.developer-mode", "true");
  let resolve!: (value: unknown) => void;
  await mount(vi.fn(() => new Promise(resolveValue => { resolve = resolveValue; })));
  await click("查看原始数据"); await click("关闭");
  await act(async () => resolve({ late: true })); expect(document.querySelector("dialog")).toBeNull();
  await click("查看原始数据");
  await act(async () => window.dispatchEvent(new StorageEvent("storage", { key: "research-assistant.developer-mode", newValue: "false" })));
  expect(document.querySelector("dialog")).toBeNull(); expect(document.querySelector(".agent-message-data-trigger")).toBeNull();
});

it("inspects command records in developer mode without adding regular message metadata", async () => {
  const record: AgentCompactionRecord = { id: "compact/1", conversationId: "conversation", state: "pending" };
  const result = { beforeTokens: 82000, usage: { inputTokens: 18000, contextWindowTokens: 128000 } };
  async function render(value: AgentCompactionRecord) {
    await act(async () => root.render(<DeveloperModeProvider><DeveloperModeToggle /><AgentCompactionActivity records={[value]} /></DeveloperModeProvider>));
  }
  await render(record);
  expect(document.querySelector('.agent-compaction-activity')!.textContent).toBe("正在压缩上下文");
  expect(document.querySelector('.agent-compaction-activity button')).toBeNull();
  await act(async () => document.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click());
  await click("查看原始数据");
  expect(JSON.parse(document.querySelector('dialog pre')!.textContent!)).toEqual({ command: "/compact", ...record });
  await render({ ...record, state: "completed", result });
  // An open inspection holds its original snapshot while the row updates.
  expect(JSON.parse(document.querySelector('dialog pre')!.textContent!).state).toBe("pending");
  await click("关闭"); await click("查看原始数据");
  expect(JSON.parse(document.querySelector('dialog pre')!.textContent!).result).toEqual(result);
  const row = document.querySelector('.agent-compaction-activity')!;
  expect(row.textContent).toBe("上下文已压缩");
  expect(row.querySelector('time, details, [aria-label="复制消息"]')).toBeNull();
  expect(row.querySelectorAll('button')).toHaveLength(1);
});
it("toggles tool records inline for selection and removes them when developer mode is disabled", async () => {
  window.localStorage.setItem("research-assistant.developer-mode", "true");
  const data = { call: { name: "research_apply_patch", content: "original patch" }, result: { error: "conflict" } };
  await act(async () => root.render(<DeveloperModeProvider><DeveloperModeToggle /><AgentMessageData data={data} presentation="inline" /></DeveloperModeProvider>));
  expect(document.querySelector("pre")).toBeNull();
  await click("查看原始数据");
  expect(document.querySelector("dialog")).toBeNull();
  const pre = document.querySelector('pre[aria-label="消息原始数据"]')!;
  expect(JSON.parse(pre.textContent!)).toEqual(data);
  expect(document.querySelector('[aria-expanded="true"]')?.getAttribute("aria-controls")).toBe(pre.id);
  // Later receipts and merged reads must remain inspectable without reopening.
  const updated = [data, { call: { name: "research_read_source" }, result: { text: "next window" } }];
  await act(async () => root.render(<DeveloperModeProvider><DeveloperModeToggle /><AgentMessageData data={updated} presentation="inline" /></DeveloperModeProvider>));
  expect(JSON.parse(pre.textContent!)).toEqual(updated);
  expect(document.querySelector('[aria-expanded="true"]')?.getAttribute("aria-controls")).toBe(pre.id);
  await click("查看原始数据"); expect(document.querySelector("pre")).toBeNull();
  await click("查看原始数据");
  await act(async () => document.querySelector<HTMLInputElement>('input[type="checkbox"]')!.click());
  expect(document.querySelector("pre, .agent-message-data-trigger")).toBeNull();
});
