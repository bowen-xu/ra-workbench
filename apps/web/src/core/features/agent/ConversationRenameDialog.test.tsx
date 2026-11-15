/** @vitest-environment happy-dom */
import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { AgentCatalogGateway } from "@research-assistant/data/agent";
import type { AgentRenameProof, AgentRenameRequest } from "@research-assistant/core/agent";
import { renameProof, renameSummary as summary } from "./conversationRenameTestFacts";
import { getAppStorage } from "../../runtime/appStorage";
import { resetConversationRenameOwnersForTests } from "./conversationRenameController";
import { ConversationMenu } from "./ConversationMenu";
import { notify } from "../../services/notifications";
vi.mock("../../services/agentGateway", () => ({ agentConversationExportGateway: {}, agentCatalogGateway: {}, agentMetadataGateway: {} }));
vi.mock("../../services/notifications", () => ({ notify: { error: vi.fn(), success: vi.fn() } }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root;
afterEach(async () => { await act(async () => root.unmount()); resetConversationRenameOwnersForTests(getAppStorage()); getAppStorage().clear(); document.body.replaceChildren(); vi.restoreAllMocks(); vi.clearAllMocks(); });
async function mount(rename: AgentCatalogGateway["rename"], onRenamed = vi.fn(), onConflict = vi.fn()) {
  vi.spyOn(HTMLDialogElement.prototype, "showModal").mockImplementation(function (this: HTMLDialogElement) { this.open = true; this.setAttribute("open", ""); this.querySelector<HTMLInputElement>("input")?.focus(); });
  vi.spyOn(HTMLDialogElement.prototype, "close").mockImplementation(function (this: HTMLDialogElement) { this.open = false; this.removeAttribute("open"); });
  const host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  const renameStatus = vi.fn(async (request: AgentRenameRequest) => ({ schemaVersion: 1 as const, operationId: request.operationId,
    conversationId: request.id, status: { type: "committed" as const, proof: renameProof(request) } }));
  const metadata = { read: vi.fn(async () => summary) };
  await act(async () => root.render(<StrictMode><ConversationMenu conversationId={summary.id} title="原标题" catalog={{ rename, renameStatus, list: vi.fn() }} metadata={metadata} onRenamed={onRenamed} onConflict={onConflict} /></StrictMode>));
  const trigger = host.querySelector<HTMLButtonElement>("button")!;
  await act(async () => trigger.click());
  const item = [...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find((node) => node.textContent?.includes("重命名"))!;
  await act(async () => {
    item.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    document.activeElement?.dispatchEvent(new FocusEvent("focusout", { bubbles: true, relatedTarget: null }));
    item.dispatchEvent(new PointerEvent("pointerup", { bubbles: true })); item.click();
  });
  return { trigger, input: document.querySelector<HTMLInputElement>("dialog input")!, onRenamed, onConflict, renameStatus, metadata };
}
async function edit(input: HTMLInputElement, text: string) {
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, text);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}
function submit() { document.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })); }

it("owns dialog focus after menu selection, freezes expected title and submits once", async () => {
  let finish!: (value: AgentRenameProof) => void;
  const rename = vi.fn((_request: AgentRenameRequest) => new Promise<AgentRenameProof>((resolve) => { finish = resolve; }));
  const { input, trigger, onRenamed } = await mount(rename);
  expect(document.activeElement).toBe(input);
  expect(input.selectionStart).toBe(0); expect(input.selectionEnd).toBe(input.value.length);
  await edit(input, "  新标题🧪  ");
  await act(async () => { submit(); submit(); });
  expect(rename).toHaveBeenCalledExactlyOnceWith({ operationId: expect.any(String), id: summary.id, expectedTitle: "原标题", title: "新标题🧪" });
  await act(async () => document.querySelector("dialog")!.dispatchEvent(new Event("cancel", { cancelable: true })));
  expect(document.querySelector("dialog")).not.toBeNull();
  await act(async () => finish(renameProof(rename.mock.calls[0]![0])));
  expect(onRenamed).toHaveBeenCalledExactlyOnceWith(summary);
  expect(document.querySelector("dialog")).toBeNull(); expect(document.activeElement).toBe(trigger);
});

it("retains failed input, retries the same compare-and-set, and does not submit IME or invalid names", async () => {
  const rename = vi.fn<AgentCatalogGateway["rename"]>().mockRejectedValueOnce("unavailable");
  const { input, renameStatus, metadata } = await mount(rename);
  await edit(input, " "); await act(async () => submit()); expect(rename).not.toHaveBeenCalled();
  await edit(input, "文".repeat(181)); await act(async () => submit()); expect(rename).not.toHaveBeenCalled();
  await edit(input, summary.title);
  await act(async () => { input.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })); submit(); });
  expect(rename).not.toHaveBeenCalled();
  await act(async () => input.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true })));
  await act(async () => submit());
  expect(input.value).toBe(summary.title); expect(document.querySelector("dialog")).not.toBeNull();
  expect(notify.success).not.toHaveBeenCalled();
  expect(input.readOnly).toBe(true);
  await act(async () => submit()); expect(rename).toHaveBeenCalledTimes(1);
  const button = (label: string) => [...document.querySelectorAll<HTMLButtonElement>("dialog button")].find(node => node.textContent === label)!;
  await act(async () => button("核对原改名操作").click());
  expect(renameStatus).toHaveBeenCalledExactlyOnceWith(rename.mock.calls[0]![0]);
  expect(metadata.read).not.toHaveBeenCalled();
  const proof = document.querySelector<HTMLDetailsElement>("dialog details")!;
  expect(proof.open).toBe(false);
  expect(proof.querySelector("summary")?.textContent).toBe("查看保存证明");
  expect(proof.textContent).toContain("9007199254740993");
  expect(document.querySelector("dialog")?.querySelector("p")?.textContent).toContain("原改名操作已保存");
  await act(async () => button("读取当前名称").click());
  expect(rename).toHaveBeenCalledTimes(1); expect(document.querySelector("dialog")).toBeNull();
});

it("does not overwrite a conflicting title and keeps the user's draft until cancel", async () => {
  const rename = vi.fn().mockRejectedValue("conflict");
  const { input, onConflict, onRenamed } = await mount(rename);
  await edit(input, summary.title); await act(async () => submit());
  expect(onConflict).not.toHaveBeenCalled(); expect(onRenamed).not.toHaveBeenCalled();
  expect(input.value).toBe(summary.title);
  await act(async () => submit()); expect(rename).toHaveBeenCalledOnce();
  await act(async () => document.querySelector("dialog")!.dispatchEvent(new Event("cancel", { cancelable: true })));
  expect(onConflict).toHaveBeenCalledOnce();
});

it("shows safe metadata read diagnostics and retries only the independent read", async () => {
  const rename = vi.fn(async (request: AgentRenameRequest) => renameProof(request));
  const { input, metadata } = await mount(rename);
  metadata.read.mockRejectedValueOnce({ code: "METADATA_READ_UNAVAILABLE", status: 503, message: "PRIVATE_PROVIDER_BODY" });
  await edit(input, summary.title); await act(async () => submit());
  const dialog = document.querySelector("dialog")!;
  expect(dialog.textContent).toContain("METADATA_READ_UNAVAILABLE"); expect(dialog.textContent).toContain("HTTP 503");
  expect(dialog.textContent).not.toContain("PRIVATE_PROVIDER_BODY");
  await act(async () => [...dialog.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === "读取当前名称")!.click());
  expect(rename).toHaveBeenCalledTimes(1); expect(metadata.read).toHaveBeenCalledTimes(2);
  expect(document.querySelector("dialog")).toBeNull();
});

it("does not update a new host when an old request completes after unmount", async () => {
  let finish!: (value: AgentRenameProof) => void;
  let intent!: AgentRenameRequest;
  const { input, onRenamed } = await mount(request => { intent = request; return new Promise((resolve) => { finish = resolve; }); });
  await edit(input, summary.title); await act(async () => submit());
  await act(async () => root.unmount());
  await act(async () => finish(renameProof(intent)));
  expect(onRenamed).not.toHaveBeenCalled();
  expect(notify.success).toHaveBeenCalledWith(`改名操作已保存，当前名称为“${summary.title}”`);
});

it("restores focus after removing the modal even when background controls are inert", async () => {
  const closed = vi.spyOn(HTMLDialogElement.prototype, "close");
  const originalFocus = HTMLButtonElement.prototype.focus;
  vi.spyOn(HTMLButtonElement.prototype, "focus").mockImplementation(function (this: HTMLButtonElement, options?: FocusOptions) {
    if (document.querySelector("dialog[open]") && !this.closest("dialog")) return;
    originalFocus.call(this, options);
  });
  const { trigger } = await mount(vi.fn());
  const native = document.querySelector("dialog")!;
  expect(native.open).toBe(true);
  await act(async () => document.querySelector("dialog")!.dispatchEvent(new Event("cancel", { cancelable: true })));
  expect(document.querySelector("dialog")).toBeNull();
  expect(closed).toHaveBeenCalled(); expect(native.open).toBe(false);
  expect(document.activeElement).toBe(trigger);
});
