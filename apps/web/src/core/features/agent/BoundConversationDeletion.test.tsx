/** @vitest-environment happy-dom */
import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { BoundConversationList } from "./BoundConversationList";
import { shouldSkipConversationDeleteConfirmation } from "./ConversationDeleteDialog";
import { notify } from "../../services/notifications";
import type { AgentDeletionRequest } from "@research-assistant/core/agent";
import { MockDeletionReceipts } from "./MockDeletionReceipts";
import { mockWebLocks } from "../../runtime/testing/mockWebLocks";
vi.mock("../../services/agentGateway", async importOriginal => ({ ...await importOriginal<typeof import("../../services/agentGateway")>(), agentArchiveGateway: {}, agentCatalogGateway: {}, agentCreationGateway: {}, agentSearchGateway: {}, agentConversationExportGateway: {}, agentDeletionGateway: {} }));
vi.mock("../../services/notifications", () => ({ notify: { error: vi.fn(), success: vi.fn() } }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const localStorageValues = new Map<string, string>();
Object.defineProperty(window, "localStorage", { configurable: true, value: {
  get length() {return localStorageValues.size;}, key: (index: number) => [...localStorageValues.keys()][index] ?? null,
  getItem: (key: string) => localStorageValues.get(key) ?? null,
  setItem: (key: string, value: string) => { localStorageValues.set(key, value); },
  removeItem: (key: string) => { localStorageValues.delete(key); },
} });
let root: Root;
afterEach(async () => { await act(async () => root?.unmount()); document.body.replaceChildren(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.clearAllMocks(); localStorageValues.clear(); });
function button(text: string) {
  const buttons = [...document.querySelectorAll<HTMLButtonElement>("button")];
  return (buttons.find((node) => node.textContent === text) ?? buttons.find((node) => node.getAttribute("aria-label") === text))!;
}
async function retryUnknown(): Promise<void> {
  await act(async () => button("核对原删除").click());
  await act(async () => button("重试原删除").click());
}
const conversation = { id: "00000000-0000-0000-0000-000000000001", title: "Synthetic", archived: true, durableCount: 0, createdAt: "now", updatedAt: "now" };
async function mount(remove: (request: AgentDeletionRequest) => Promise<unknown>) {
  mockWebLocks();
  vi.spyOn(HTMLDialogElement.prototype, "showModal").mockImplementation(function (this: HTMLDialogElement) { this.setAttribute("open", ""); });
  const host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  let present = true;
  const list = vi.fn(async ({ archived }) => ({ schemaVersion: 1 as const, conversations: archived && present ? [conversation] : [], next: null }));
  const onSelect = vi.fn(), onDeleted = vi.fn(() => { present = false; });
  const receipts = new MockDeletionReceipts(() => "00000000-0000-4000-8000-000000000003");
  const deletions = {status: async (request: AgentDeletionRequest) => receipts.status(request),
    delete: async (request: AgentDeletionRequest, options?: {expectedEpoch: string}) => {
      receipts.validateWrite(request, options); await remove(request); receipts.apply(request, () => {present = false;});
      return {schemaVersion: 1 as const, requestId: request.requestId, conversationId: request.conversationId};
    }};
  await act(async () => root.render(<StrictMode><BoundConversationList binding={{ kind: "manuscript", resourceId: "m" }} selectedId={conversation.id}
    catalog={{ list, rename: vi.fn(), renameStatus: vi.fn() }} deletions={deletions} onSelect={onSelect} onDeleted={onDeleted} /></StrictMode>));
  await act(async () => button("查看归档对话").click());
  const trigger = host.querySelector<HTMLButtonElement>('[aria-haspopup="menu"]')!;
  await act(async () => trigger.click());
  await act(async () => {
    const item = button("永久删除对话");
    item.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    item.dispatchEvent(new FocusEvent("focusout", { bubbles: true, relatedTarget: null }));
    item.dispatchEvent(new PointerEvent("pointerup", { bubbles: true })); item.click();
  });
  return { list, onSelect, onDeleted, trigger };
}
it("requires confirmation, owns dialog focus and cancels without mutation", async () => {
  const remove = vi.fn(); const { onSelect } = await mount(remove);
  expect(document.activeElement).toBe(button("取消"));
  expect(document.querySelector("dialog h2")?.textContent).toBe("永久删除对话");
  expect(document.querySelector("dialog > p")).toBeNull();
  expect(button("确认").className).toBe("primary-button");
  expect(button("取消").className).toBe("ghost-button");
  expect(remove).not.toHaveBeenCalled(); expect(onSelect).not.toHaveBeenCalled();
  await act(async () => document.querySelector("dialog")!.dispatchEvent(new Event("cancel", { cancelable: true })));
  expect(document.querySelector("dialog")).toBeNull(); expect(document.activeElement).toBe(button("永久删除对话"));
});
it("allows suppressing the confirmation for the rest of the local day", async () => {
  localStorageValues.clear();
  const remove = vi.fn().mockResolvedValue({});
  await mount(remove);
  const checkbox = document.querySelector<HTMLInputElement>(".conversation-delete-skip input")!;
  await act(async () => { checkbox.click(); button("确认").click(); });
  expect(remove).toHaveBeenCalledOnce();
  expect(shouldSkipConversationDeleteConfirmation()).toBe(true);
  await act(async () => root.unmount());
  document.body.replaceChildren();
  await mount(remove);
  expect(remove).toHaveBeenCalledTimes(2);
  expect(document.querySelector("dialog")).toBeNull();
  localStorageValues.clear();
});
it("keeps one frozen request on uncertain retry and focuses the surviving filter on success", async () => {
  let finish!: (value: unknown) => void;
  const remove = vi.fn().mockRejectedValueOnce("unavailable").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
  const { onDeleted, onSelect } = await mount(remove);
  await act(async () => button("确认").click());
  expect(onDeleted).not.toHaveBeenCalled(); expect(notify.success).not.toHaveBeenCalled();
  await act(async () => button("核对原删除").click());
  await act(async () => { const retry = button("重试原删除"); retry.click(); retry.click(); });
  expect(remove).toHaveBeenCalledTimes(2); expect(remove.mock.calls[0]).toEqual(remove.mock.calls[1]);
  await act(async () => document.querySelector("dialog")!.dispatchEvent(new Event("cancel", { cancelable: true })));
  expect(document.querySelector("dialog")).not.toBeNull();
  await act(async () => finish({}));
  expect(onDeleted).toHaveBeenCalledExactlyOnceWith(conversation.id);
  expect(onSelect).not.toHaveBeenCalled(); expect(document.querySelector("dialog")).toBeNull();
  expect(document.activeElement).toBe(button("返回当前对话"));
  expect(notify.error).toHaveBeenCalledExactlyOnceWith(expect.any(String));
  expect(notify.success).toHaveBeenCalledExactlyOnceWith(expect.stringContaining("Synthetic"));
});
it("restores a checked preference when a suppressed ordinary confirmation reappears after failure", async () => {
  const today = new Date();
  localStorageValues.set("research-assistant.agent.delete-confirmation-skip-date", `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(today.getDate()).padStart(2, "0")}`);
  const remove = vi.fn().mockRejectedValueOnce("unavailable").mockResolvedValueOnce({});
  await mount(remove);
  expect(remove).toHaveBeenCalledOnce();
  expect(document.querySelector('dialog[open]')).not.toBeNull();
  expect(document.querySelector('[role="alert"]')?.textContent).toContain("请先核对原请求");
  expect(document.querySelector<HTMLInputElement>('.conversation-delete-skip input')!.checked).toBe(true);
  await retryUnknown();
  expect(remove.mock.calls[0]).toEqual(remove.mock.calls[1]);
});
it("persists a newly checked preference across failed retries, cancel and reopening the list", async () => {
  const remove = vi.fn().mockRejectedValue("busy");
  await mount(remove);
  await act(async () => document.querySelector<HTMLInputElement>('.conversation-delete-skip input')!.click());
  expect(shouldSkipConversationDeleteConfirmation()).toBe(true);
  expect(remove).not.toHaveBeenCalled();
  await act(async () => button("确认").click());
  await act(async () => button("重试").click());
  await act(async () => button("取消").click());
  expect(shouldSkipConversationDeleteConfirmation()).toBe(true);
  await act(async () => root.unmount()); document.body.replaceChildren();
  await mount(remove);
  expect(remove).toHaveBeenCalledTimes(3);
  expect(document.querySelector<HTMLInputElement>('.conversation-delete-skip input')!.checked).toBe(true);
  expect(button("重试").disabled).toBe(false);
  expect(remove.mock.calls[0]).toEqual(remove.mock.calls[1]);
  expect(remove.mock.calls[2]?.[0].requestId).not.toBe(remove.mock.calls[0]?.[0].requestId);
});
it.each(["branchHistory", "proposalApplications", "executionWorkspace", "conflict"])("retains protected histories for %s without a blind retry", async (reason) => {
  const remove = vi.fn().mockRejectedValue(reason); const { onDeleted } = await mount(remove);
  await act(async () => button("确认").click());
  if (reason === "conflict") expect(button("重试").disabled).toBe(true);
  else { expect(document.querySelector('dialog[aria-label="强制删除对话"]')).not.toBeNull(); expect(button("确认").disabled).toBe(false); }
  expect(button("取消").disabled).toBe(false);
  if (reason === "conflict") expect(document.querySelector('[role="alert"]')?.textContent).toContain("自动核对列表");
  expect(onDeleted).not.toHaveBeenCalled();
  if (reason === "conflict") expect(notify.error).toHaveBeenCalledExactlyOnceWith(expect.any(String));
  else expect(notify.error).not.toHaveBeenCalled();
});
it("offers a separately confirmed force deletion for retained history", async () => {
  const remove = vi.fn().mockRejectedValueOnce("executionWorkspace").mockResolvedValueOnce({});
  const { onDeleted } = await mount(remove);
  await act(async () => button("确认").click());
  expect(document.querySelector('dialog[aria-label="永久删除对话"]')).toBeNull();
  expect(document.querySelector('dialog[aria-label="强制删除对话"]')).not.toBeNull();
  expect(document.activeElement).toBe(button("取消"));
  expect(remove).toHaveBeenCalledOnce();
  await act(async () => button("确认").click());
  expect(remove).toHaveBeenCalledTimes(2);
  expect(remove.mock.calls[0]?.[0]).toMatchObject({ force: false });
  expect(remove.mock.calls[1]?.[0]).toMatchObject({ force: true });
  expect(remove.mock.calls[1]?.[0]).toEqual({ ...remove.mock.calls[0]?.[0], force: true });
  expect(onDeleted).toHaveBeenCalledExactlyOnceWith(conversation.id);
  expect(notify.error).not.toHaveBeenCalled();
  expect(notify.success).toHaveBeenCalledExactlyOnceWith(expect.stringContaining("Synthetic"));
});
it("does not update another host after the deletion completes", async () => {
  let finish!: (value: unknown) => void;
  const remove = vi.fn(() => new Promise((resolve) => { finish = resolve; }));
  const { onDeleted } = await mount(remove);
  await act(async () => button("确认").click());
  await act(async () => root.render(<button>其他稿件</button>));
  button("其他稿件").focus();
  await act(async () => finish({}));
  expect(onDeleted).not.toHaveBeenCalled(); expect(document.activeElement).toBe(button("其他稿件"));
  expect(notify.success).toHaveBeenCalledWith(expect.stringContaining("Synthetic"));
});

it("persists force suppression separately and expires it on the next local day", async () => {
  const remove = vi.fn().mockRejectedValueOnce("branchHistory").mockResolvedValueOnce({});
  await mount(remove);
  await act(async () => button("确认").click());
  await act(async () => {
    document.querySelector<HTMLInputElement>('.conversation-delete-skip input')!.click();
    button("确认").click();
  });
  const key = "research-assistant.agent.force-delete-confirmation-skip-date";
  expect(localStorageValues.has(key)).toBe(true);
  expect(shouldSkipConversationDeleteConfirmation()).toBe(false);
  await act(async () => root.unmount()); document.body.replaceChildren();
  localStorageValues.set(key, "2000-01-01");
  await mount(vi.fn().mockRejectedValue("branchHistory"));
  await act(async () => button("确认").click());
  expect(document.querySelector('dialog[aria-label="强制删除对话"]')).not.toBeNull();
});

it("retains both independent preferences through failed force requests and retries", async () => {
  const remove = vi.fn().mockRejectedValueOnce("branchHistory").mockRejectedValueOnce("unavailable").mockResolvedValueOnce({});
  await mount(remove);
  await act(async () => {
    document.querySelector<HTMLInputElement>('.conversation-delete-skip input')!.click();
    button("确认").click();
  });
  await act(async () => {
    document.querySelector<HTMLInputElement>('.conversation-delete-skip input')!.click();
    button("确认").click();
  });
  expect(localStorageValues.has("research-assistant.agent.force-delete-confirmation-skip-date")).toBe(true);
  expect(shouldSkipConversationDeleteConfirmation()).toBe(true);
  expect(document.querySelector('dialog[aria-label="强制删除对话"]')).not.toBeNull();
  await retryUnknown();
  expect(remove.mock.calls[1]).toEqual(remove.mock.calls[2]);
  expect(remove.mock.calls[1]?.[0]).toMatchObject({ force: true });
  expect(localStorageValues.has("research-assistant.agent.force-delete-confirmation-skip-date")).toBe(true);
  expect(shouldSkipConversationDeleteConfirmation()).toBe(true);
});

it("shows force errors after suppressed confirmation instead of silently closing the flow", async () => {
  const today = new Date();
  localStorageValues.set("research-assistant.agent.force-delete-confirmation-skip-date", `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(today.getDate()).padStart(2, "0")}`);
  const remove = vi.fn().mockRejectedValueOnce("branchHistory").mockRejectedValueOnce("busy");
  await mount(remove); await act(async () => button("确认").click());
  expect(remove).toHaveBeenCalledTimes(2);
  expect(document.querySelector('dialog[aria-label="强制删除对话"]')?.hasAttribute("open")).toBe(true);
  expect(document.activeElement).toBe(button("取消"));
  expect(button("重试").disabled).toBe(false);
  expect(document.querySelector<HTMLInputElement>('.conversation-delete-skip input')!.checked).toBe(true);
  expect(document.querySelector('[role="alert"]')?.textContent).toContain("运行或恢复任务");
  expect(notify.error).toHaveBeenCalledExactlyOnceWith(expect.stringContaining("运行或恢复任务"));
});

it("keeps the original delete request and shows matching storage diagnostics on an uncertain native result", async () => {
  const remove = vi.fn(async (request: AgentDeletionRequest) => { throw {code:"storage",status:503,reason:{code:"DATA_SQL_COMMIT_UNKNOWN",
    reason:{operationId:request.requestId,phase:"commit",safeCause:"sql_commit_outcome_unknown",sqliteExtendedCode:787,sql:"private SQL"}}}; });
  const { onDeleted } = await mount(remove);
  await act(async () => button("确认").click());
  const message = vi.mocked(notify.error).mock.calls.at(-1)![0];
  expect(message).toContain("DATA_SQL_COMMIT_UNKNOWN"); expect(message).toContain(remove.mock.calls[0]![0].requestId);
  expect(message).toContain("787"); expect(message).toContain("HTTP 503"); expect(message).not.toContain("private SQL");
  await retryUnknown();
  expect(remove.mock.calls[1]).toEqual(remove.mock.calls[0]); expect(onDeleted).not.toHaveBeenCalled();
  expect(document.querySelector("dialog")).not.toBeNull();
});
