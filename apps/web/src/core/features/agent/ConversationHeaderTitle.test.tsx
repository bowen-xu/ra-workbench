/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { ConversationHeaderTitle } from "./ConversationHeaderTitle";
import type { AgentRenameRequest, AgentRenameProof } from "@research-assistant/core/agent";
import { renameProof, renameSummary as summary } from "./conversationRenameTestFacts";
import { getAppStorage } from "../../runtime/appStorage";
import { resetConversationRenameOwnersForTests } from "./conversationRenameController";
import { nameInitialConversation } from "./automaticConversationName";
vi.mock("../../services/notifications", () => ({notify: {success: vi.fn(), error: vi.fn()}}));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root;
afterEach(async () => {await act(async () => root.unmount()); resetConversationRenameOwnersForTests(getAppStorage()); getAppStorage().clear(); document.body.replaceChildren(); vi.restoreAllMocks();});
async function mount(editable = true) {
  vi.spyOn(HTMLDialogElement.prototype, "showModal").mockImplementation(function (this: HTMLDialogElement) { this.open = true; this.setAttribute("open", ""); });
  vi.spyOn(HTMLDialogElement.prototype, "close").mockImplementation(function (this: HTMLDialogElement) { this.open = false; this.removeAttribute("open"); });
  const rename = vi.fn(async (request: AgentRenameRequest) => renameProof(request)), onRenamed = vi.fn(), onConflict = vi.fn();
  const metadata = { read: vi.fn(async () => summary) };
  const renameStatus = vi.fn(async (request: AgentRenameRequest) => ({ schemaVersion: 1 as const, operationId: request.operationId,
    conversationId: request.id, status: { type: "committed" as const, proof: renameProof(request) } }));
  const host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root.render(<ConversationHeaderTitle title="原标题" {...(editable ? {rename: {conversationId: summary.id, catalog: {rename, renameStatus}, metadata, onRenamed, onConflict}} : {})}/>));
  return {host, title: host.querySelector("span")!, rename, renameStatus, metadata, onRenamed, onConflict};
}
async function begin(title: HTMLElement): Promise<HTMLInputElement> {
  await act(async () => title.dispatchEvent(new MouseEvent("dblclick", {bubbles: true})));
  return document.querySelector<HTMLInputElement>('[aria-label="对话名称"]')!;
}
async function fill(input: HTMLInputElement, value: string): Promise<void> {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
    input.dispatchEvent(new Event("input", {bubbles: true}));
  });
}
async function key(input: HTMLInputElement, value: string): Promise<KeyboardEvent> {
  const event = new KeyboardEvent("keydown", {key: value, bubbles: true, cancelable: true});
  await act(async () => {
    input.dispatchEvent(event);
  });
  return event;
}
it("edits in place with selected text and saves on Tab without duplicate blur submission", async () => {
  const {title, host, rename, onRenamed} = await mount();
  const input = await begin(title);
  expect(document.activeElement).toBe(input);
  expect([input.selectionStart, input.selectionEnd]).toEqual([0, input.value.length]);
  expect(document.querySelector("dialog")).toBeNull();
  expect(host.querySelector("button")).toBeNull();
  await fill(input, ` ${summary.title} `);
  let finish!: (value: AgentRenameProof) => void;
  rename.mockImplementationOnce(() => new Promise(resolve => {finish = resolve;}));
  const tab = await key(input, "Tab");
  expect(tab.defaultPrevented).toBe(true);
  expect(document.activeElement).toBe(input);
  await act(async () => input.dispatchEvent(new FocusEvent("focusout", {bubbles: true})));
  expect(rename).toHaveBeenCalledExactlyOnceWith({operationId: expect.any(String), id: summary.id, expectedTitle: "原标题", title: summary.title});
  await act(async () => finish(renameProof(rename.mock.calls[0]![0])));
  expect(onRenamed).toHaveBeenCalledExactlyOnceWith(summary);
  expect(host.querySelector("input")).toBeNull();
  expect(document.activeElement).toBe(host.querySelector("span"));
});
it("saves on blur and leaves focus on the clicked control", async () => {
  const {title, rename} = await mount();
  const input = await begin(title); await fill(input, summary.title);
  const other = document.createElement("button"); document.body.append(other);
  await act(async () => other.focus());
  expect(rename).toHaveBeenCalledOnce(); expect(document.activeElement).toBe(other);
});
it("supports keyboard activation and Escape cancellation without a blur save", async () => {
  const {title, host, rename} = await mount();
  await act(async () => title.dispatchEvent(new KeyboardEvent("keydown", {key: "F2", bubbles: true, cancelable: true})));
  const input = host.querySelector("input")!; await fill(input, "取消修改");
  await key(input, "Escape");
  expect(host.querySelector("input")).toBeNull(); expect(rename).not.toHaveBeenCalled();
  expect(document.activeElement).toBe(host.querySelector("span"));
  expect(host.textContent).toBe("原标题");
});
it("does not create or rename a pending conversation when double clicked", async () => {
  const {title, host, rename} = await mount(false);
  await act(async () => title.dispatchEvent(new MouseEvent("dblclick", {bubbles: true})));
  expect(title.getAttribute("role")).toBeNull(); expect(host.querySelector("input")).toBeNull();
  expect(rename).not.toHaveBeenCalled();
});
it("does not add a recovery button while automatic naming is in progress", async () => {
  const {host, rename, renameStatus, metadata} = await mount();
  let finish!: (proof: AgentRenameProof) => void;
  rename.mockImplementationOnce(() => new Promise(resolve => {finish = resolve;}));
  let naming!: Promise<unknown>;
  await act(async () => {
    naming = nameInitialConversation(summary.id, crypto.randomUUID(), "首条消息", {catalog: {rename, renameStatus}, metadata});
  });
  expect(rename).toHaveBeenCalledOnce();
  expect(host.querySelector("button")).toBeNull();
  await act(async () => host.querySelector<HTMLSpanElement>("span")!.dispatchEvent(new MouseEvent("dblclick", {bubbles: true})));
  expect(document.querySelector("dialog")).toBeNull();
  await act(async () => {finish(renameProof(rename.mock.calls[0]![0])); await naming;});
  expect(host.querySelector("button")).toBeNull();
});
it.each([" ", "文".repeat(181), "名称\0"])("retains invalid input without saving: %s", async value => {
  const {title, rename} = await mount(); const input = await begin(title);
  await fill(input, value); await key(input, "Tab");
  expect(rename).not.toHaveBeenCalled(); expect(input.value).toBe(value);
  expect(input.getAttribute("aria-invalid")).toBe("true");
  await fill(input, summary.title); await key(input, "Tab");
  expect(rename).toHaveBeenCalledOnce();
});
it("locks an unknown intent and explicitly inspects status before reading the current title", async () => {
  const {title, rename, renameStatus, metadata, onRenamed} = await mount(); const input = await begin(title);
  rename.mockRejectedValueOnce("offline"); await fill(input, summary.title); await key(input, "Tab");
  const recovered = document.querySelector<HTMLInputElement>("dialog input")!;
  expect(recovered.value).toBe(summary.title); expect(recovered.readOnly).toBe(true);
  const button = (text: string) => [...document.querySelectorAll("dialog button")].find(node => node.textContent === text) as HTMLButtonElement;
  await act(async () => button("核对原改名操作").click());
  expect(renameStatus).toHaveBeenCalledExactlyOnceWith(rename.mock.calls[0]![0]);
  expect(metadata.read).not.toHaveBeenCalled();
  await act(async () => button("读取当前名称").click());
  expect(rename).toHaveBeenCalledTimes(1);
  expect(onRenamed).toHaveBeenCalledExactlyOnceWith(summary);
});
it("retries only metadata after a committed rename fails to read the current name", async () => {
  const {title, rename, metadata} = await mount(); const input = await begin(title);
  metadata.read.mockRejectedValueOnce(new Error("read failed"));
  await fill(input, summary.title); await key(input, "Tab");
  expect(rename).toHaveBeenCalledTimes(1);
  const read = [...document.querySelectorAll<HTMLButtonElement>("dialog button")].find(node => node.textContent === "读取当前名称")!;
  await act(async () => read.click());
  expect(metadata.read).toHaveBeenCalledTimes(2);
  expect(rename).toHaveBeenCalledTimes(1);
});
it("keeps editing on Enter without saving or submitting a parent form", async () => {
  const {title, rename} = await mount(); const input = await begin(title);
  await fill(input, summary.title);
  const enter = await key(input, "Enter");
  expect(enter.defaultPrevented).toBe(true);
  expect(rename).not.toHaveBeenCalled();
  expect(input.isConnected).toBe(true); expect(document.activeElement).toBe(input);
});
it("blocks conflict overwrite until cancellation refreshes metadata", async () => {
  const {title, rename, onConflict} = await mount(); const input = await begin(title);
  rename.mockRejectedValueOnce("conflict"); await fill(input, summary.title); await key(input, "Tab");
  await key(input, "Tab"); expect(rename).toHaveBeenCalledOnce();
  await key(input, "Escape"); expect(onConflict).toHaveBeenCalledOnce();
});
it("does not save or cancel during IME composition", async () => {
  const {title, rename} = await mount(); const input = await begin(title); await fill(input, summary.title);
  await act(async () => input.dispatchEvent(new Event("compositionstart", {bubbles: true})));
  expect((await key(input, "Enter")).defaultPrevented).toBe(false);
  await key(input, "Escape"); expect((await key(input, "Tab")).defaultPrevented).toBe(true);
  expect(rename).not.toHaveBeenCalled(); expect(input.isConnected).toBe(true);
  await act(async () => input.dispatchEvent(new Event("compositionend", {bubbles: true})));
  await key(input, "Tab"); expect(rename).toHaveBeenCalledOnce();
});
it("ignores an old conversation's UI callback after the host is replaced", async () => {
  const {title, rename, onRenamed, host} = await mount(); const input = await begin(title);
  let finish!: (value: AgentRenameProof) => void;
  rename.mockImplementationOnce(() => new Promise(resolve => {finish = resolve;}));
  await fill(input, summary.title); await key(input, "Tab");
  await act(async () => root.render(<ConversationHeaderTitle key="other" title="另一对话"/>));
  await act(async () => finish(renameProof(rename.mock.calls[0]![0])));
  expect(onRenamed).not.toHaveBeenCalled(); expect(host.textContent).toBe("另一对话");
});

it("returns keyboard focus to the live header after closing unknown recovery without repeating rename", async () => {
  const closed = vi.spyOn(HTMLDialogElement.prototype, "close");
  const { title, host, rename } = await mount();
  await act(async () => title.dispatchEvent(new KeyboardEvent("keydown", { key: "F2", bubbles: true, cancelable: true })));
  const inline = host.querySelector("input")!;
  await fill(inline, summary.title);
  rename.mockRejectedValueOnce(new Error("reply lost"));
  await key(inline, "Tab");
  const native = document.querySelector("dialog")!;
  const recovery = native.querySelector<HTMLInputElement>("input")!;
  expect(native.open).toBe(true);
  expect(inline.isConnected).toBe(false);
  expect(document.activeElement).toBe(recovery);
  await act(async () => native.dispatchEvent(new Event("cancel", { cancelable: true })));
  expect(closed).toHaveBeenCalled();
  expect(native.open).toBe(false);
  expect(native.isConnected).toBe(false);
  expect(document.activeElement).toBe(host.querySelector("span"));
  await act(async () => host.querySelector<HTMLSpanElement>("span")!.dispatchEvent(new MouseEvent("dblclick", {bubbles: true})));
  expect(document.querySelector<HTMLInputElement>("dialog input")?.readOnly).toBe(true);
  expect(rename).toHaveBeenCalledTimes(1);
});
