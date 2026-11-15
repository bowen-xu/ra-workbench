/** @vitest-environment happy-dom */
import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { BoundConversationList } from "./BoundConversationList";
import { notify, type NotificationOptions } from "../../services/notifications";
import type { AgentArchiveProof, AgentCatalogPage, AgentConversationSummary } from "@research-assistant/core/agent";
import type { AgentArchiveGateway, AgentCatalogGateway, AgentMetadataGateway } from "@research-assistant/data/agent";
import { archiveProof } from "./conversationArchiveTestFacts";
import { resetConversationArchiveOwnersForTests } from "./conversationArchiveController";
import { readConversationArchive } from "./conversationArchivePending";
import { getAppStorage } from "../../runtime/appStorage";
import * as appStorage from "../../runtime/appStorage";
vi.mock("../../services/agentGateway", async importOriginal => ({ ...await importOriginal<typeof import("../../services/agentGateway")>(), agentArchiveGateway: {}, agentCatalogGateway: {}, agentCreationGateway: {}, agentDeletionGateway: {}, agentSearchGateway: {}, agentConversationExportGateway: {} }));
vi.mock("../../services/notifications", () => ({ notify: { error: vi.fn(), success: vi.fn(), dismiss: vi.fn() } }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const conversation: AgentConversationSummary = { id: "11111111-1111-4111-8111-111111111111", title: "Synthetic", archived: false, durableCount: 14, createdAt: "2026-09-10T00:00:00.000Z", updatedAt: "2026-09-10T00:00:00.000Z" };
const page = (items: readonly AgentConversationSummary[]): AgentCatalogPage => ({ schemaVersion: 1, conversations: items, next: null });
let root: Root;
afterEach(async () => { await act(async () => root.unmount()); resetConversationArchiveOwnersForTests(getAppStorage()); getAppStorage().clear(); document.body.replaceChildren(); vi.clearAllMocks(); });
function button(text: string) {
  const buttons = [...document.querySelectorAll<HTMLButtonElement>("button")];
  return (buttons.find((node) => node.textContent === text) ?? buttons.find((node) => node.getAttribute("aria-label") === text))!;
}
async function mount(list: AgentCatalogGateway["list"], setArchived: AgentArchiveGateway["setArchived"],
  read: AgentMetadataGateway["read"] = async () => ({ ...conversation, archived: true }), archiveStatus: AgentArchiveGateway["archiveStatus"] = vi.fn()) {
  const host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  const onSelect = vi.fn(), onArchiveChange = vi.fn();
  await act(async () => root.render(<StrictMode><BoundConversationList binding={{ kind: "manuscript", resourceId: "draft" }} selectedId={null}
    onSelect={onSelect} onArchiveChange={onArchiveChange} archives={{ setArchived, archiveStatus }} metadata={{read}} catalog={{ list, rename: vi.fn(), renameStatus: vi.fn() }} /></StrictMode>));
  return { onSelect, onArchiveChange };
}
async function menuAction(label: string) {
  await act(async () => document.querySelector<HTMLButtonElement>('[aria-haspopup="menu"]')!.click());
  await act(async () => { const item = button(label); item.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true })); item.dispatchEvent(new FocusEvent("blur", { relatedTarget: null })); item.dispatchEvent(new PointerEvent("pointerup", { bubbles: true })); item.click(); item.click(); });
}

it("archives and restores without selecting a conversation or replaying tasks", async () => {
  let archived = false;
  const list = vi.fn(async (request) => page(request.archived === archived ? [{ ...conversation, archived }] : []));
  const setArchived = vi.fn<AgentArchiveGateway["setArchived"]>(async request => { archived = request.archived; return archiveProof(request); });
  const read = vi.fn<AgentMetadataGateway["read"]>(async () => ({ ...conversation, archived }));
  const { onSelect, onArchiveChange } = await mount(list, setArchived, read);
  await menuAction("归档对话");
  expect(setArchived).toHaveBeenCalledExactlyOnceWith({ operationId: expect.any(String), id: conversation.id, archived: true, expectedWatermark: 14 });
  const original = setArchived.mock.calls[0]![0];
  expect(read).toHaveBeenCalledExactlyOnceWith({ id: conversation.id, binding: {kind: "manuscript", resourceId: "draft"} });
  expect(onArchiveChange).toHaveBeenCalledWith(conversation.id, true); expect(onSelect).not.toHaveBeenCalled();
  expect(button("Synthetic")).toBeUndefined();
  await act(async () => button("查看归档对话").click());
  expect(list).toHaveBeenLastCalledWith({ archived: true, limit: 50, before: null, binding: { kind: "manuscript", resourceId: "draft" } });
  await menuAction("恢复对话");
  expect(setArchived).toHaveBeenLastCalledWith({ operationId: expect.any(String), id: conversation.id, archived: false, expectedWatermark: null });
  expect(setArchived.mock.calls[1]![0].operationId).not.toBe(original.operationId);
  expect(onArchiveChange).toHaveBeenLastCalledWith(conversation.id, false); expect(onSelect).not.toHaveBeenCalled();
});

it("does not display a recovery error while a successful archive is reading its current state", async () => {
  let finishRead!: (current: AgentConversationSummary) => void;
  const setArchived = vi.fn<AgentArchiveGateway["setArchived"]>(async request => archiveProof(request));
  const read = vi.fn<AgentMetadataGateway["read"]>(() => new Promise(resolve => { finishRead = resolve; }));
  await mount(vi.fn(async () => page([conversation])), setArchived, read);
  await act(async () => button("归档对话").click());
  expect(read).toHaveBeenCalledOnce();
  expect(notify.error).not.toHaveBeenCalled();
  expect(notify.success).not.toHaveBeenCalled();
  await act(async () => finishRead({ ...conversation, archived: true }));
  expect(notify.error).not.toHaveBeenCalled();
  expect(notify.success).toHaveBeenCalledOnce();
  expect(vi.mocked(notify.success).mock.calls[0]![0]).toContain("当前已归档");
});

it("keeps a failed current-state read recoverable and does not dismiss its eventual success notice", async () => {
  const setArchived = vi.fn<AgentArchiveGateway["setArchived"]>(async request => archiveProof(request));
  const read = vi.fn<AgentMetadataGateway["read"]>()
    .mockRejectedValueOnce(new Error("read unavailable"))
    .mockResolvedValueOnce({ ...conversation, archived: true });
  await mount(vi.fn(async () => page([conversation])), setArchived, read);
  await act(async () => button("归档对话").click());
  expect(notify.success).not.toHaveBeenCalled();
  const options = vi.mocked(notify.error).mock.calls.at(-1)![1] as NotificationOptions;
  const action = options.actions!.find(item => item.id === "read");
  expect(action?.label).toBe("读取当前归档状态");
  expect(options.duration).toBe(Infinity);
  await act(async () => action!.onClick());
  expect(setArchived).toHaveBeenCalledOnce();
  expect(read).toHaveBeenCalledTimes(2);
  expect(notify.success).toHaveBeenCalledOnce();
  const success = vi.mocked(notify.success).mock.calls.at(-1)![1] as NotificationOptions | undefined;
  expect(success?.id).not.toBe(options.id);
  expect(success?.duration).toBeUndefined();
  expect(vi.mocked(notify.dismiss).mock.calls.some(([id]) => id === success?.id)).toBe(false);
});

it("keeps a busy conversation and reports the reason without retry", async () => {
  const setArchived = vi.fn().mockRejectedValue("busy");
  const { onArchiveChange } = await mount(vi.fn(async () => page([conversation])), setArchived);
  await menuAction("归档对话");
  expect(setArchived).toHaveBeenCalledOnce(); expect(onArchiveChange).not.toHaveBeenCalled();
  expect(button("Synthetic")).toBeDefined(); expect(notify.success).not.toHaveBeenCalled();
  expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("未结束任务"), expect.any(Object));
  expect(vi.mocked(notify.error).mock.calls.at(-1)![0]).toContain(conversation.id);
  expect(vi.mocked(notify.error).mock.calls.at(-1)![0]).toContain(setArchived.mock.calls[0]![0].operationId);
});

it("ignores a late filter page and opens archived history with explicit read-only metadata", async () => {
  let finish!: (value: AgentCatalogPage) => void;
  const list = vi.fn((request) => request.archived ? new Promise<AgentCatalogPage>((resolve) => { finish = resolve; }) : Promise.resolve(page([conversation])));
  const { onSelect } = await mount(list, vi.fn());
  await act(async () => button("查看归档对话").click());
  await act(async () => button("返回当前对话").click());
  await act(async () => finish(page([{ ...conversation, title: "Archived", archived: true }])));
  expect(button("Archived")).toBeUndefined(); expect(button("Synthetic")).toBeDefined();
  await act(async () => button("查看归档对话").click());
  await act(async () => finish(page([{ ...conversation, title: "Archived", archived: true }])));
  await act(async () => button("Archived").click());
  expect(onSelect).toHaveBeenCalledExactlyOnceWith(conversation.id, true);
});

it("finishes the original archive after a filter switch but never updates an unmounted host", async () => {
  let finish!: (value: AgentArchiveProof) => void;
  const setArchived = vi.fn<AgentArchiveGateway["setArchived"]>(() => new Promise((resolve) => { finish = resolve; }));
  const { onArchiveChange } = await mount(vi.fn(async (request) => page(request.archived ? [] : [conversation])), setArchived);
  await menuAction("归档对话");
  await act(async () => button("查看归档对话").click());
  await act(async () => finish(archiveProof(setArchived.mock.calls[0]![0])));
  expect(onArchiveChange).toHaveBeenCalledExactlyOnceWith(conversation.id, true);
  await act(async () => button("返回当前对话").click());
  await menuAction("归档对话");
  onArchiveChange.mockClear();
  vi.mocked(notify.success).mockClear(); vi.mocked(notify.error).mockClear();
  await act(async () => root.unmount());
  await act(async () => finish(archiveProof(setArchived.mock.calls[1]![0])));
  expect(onArchiveChange).not.toHaveBeenCalled();
  expect(notify.success).not.toHaveBeenCalled(); expect(notify.error).not.toHaveBeenCalled();
});

it("finds afterCommit Unknown when the row disappeared, inspects only, then reads current state explicitly", async () => {
  let committed = false;
  const setArchived = vi.fn<AgentArchiveGateway["setArchived"]>(async () => { committed = true; throw new Error("lost reply"); });
  const list = vi.fn(async () => page(committed ? [] : [conversation]));
  const read = vi.fn<AgentMetadataGateway["read"]>(async () => ({ ...conversation, title: "Later restored", archived: false, durableCount: 18 }));
  const archiveStatus = vi.fn<AgentArchiveGateway["archiveStatus"]>(async intent => ({schemaVersion: 1, operationId: intent.operationId,
    conversationId: intent.id, status: {type: "committed", proof: archiveProof(intent)}}));
  await mount(list, setArchived, read, archiveStatus); await menuAction("归档对话");
  const intent = setArchived.mock.calls[0]![0]; expect(read).not.toHaveBeenCalled();
  await act(async () => root.unmount()); resetConversationArchiveOwnersForTests(getAppStorage()); vi.mocked(notify.error).mockClear();
  const ui = await mount(list, setArchived, read, archiveStatus);
  expect(button("Synthetic")).toBeUndefined(); expect(notify.error).toHaveBeenCalledWith(expect.stringContaining(intent.operationId), expect.any(Object));
  expect(archiveStatus).not.toHaveBeenCalled();
  async function recoveryAction(label: string) {
    const options = vi.mocked(notify.error).mock.calls.at(-1)?.[1] as NotificationOptions;
    const action = options.actions!.find(action => action.label === label)!;
    await act(async () => action.onClick());
  }
  await recoveryAction("核对原归档操作"); expect(archiveStatus).toHaveBeenCalledExactlyOnceWith(intent); expect(read).not.toHaveBeenCalled();
  await recoveryAction("读取当前归档状态"); expect(read).toHaveBeenCalledExactlyOnceWith({id: intent.id, binding: {kind: "manuscript", resourceId: "draft"}});
  expect(ui.onArchiveChange).toHaveBeenCalledExactlyOnceWith(intent.id, false); expect(setArchived).toHaveBeenCalledTimes(1);
});

it("allows closing an unresolved archive notice without losing its original operation", async () => {
  const setArchived = vi.fn<AgentArchiveGateway["setArchived"]>().mockRejectedValue(new Error("lost reply"));
  const archiveStatus = vi.fn<AgentArchiveGateway["archiveStatus"]>(async intent => ({schemaVersion: 1, operationId: intent.operationId,
    conversationId: intent.id, status: {type: "noSqlReceipt", epoch: "22222222-2222-4222-8222-222222222222", sqlFence: "8", closed: false}}));
  const list = vi.fn(async () => page([conversation]));
  await mount(list, setArchived, undefined, archiveStatus);
  await menuAction("归档对话");
  const intent = setArchived.mock.calls[0]![0];
  let options = vi.mocked(notify.error).mock.calls.at(-1)![1] as NotificationOptions;
  await act(async () => options.actions!.find(action => action.id === "inspect")!.onClick());
  options = vi.mocked(notify.error).mock.calls.at(-1)![1] as NotificationOptions;
  expect(options.closeButton).toBe(true); expect(options.dismissible).not.toBe(false); expect(options.duration).toBe(Infinity);
  expect(vi.mocked(notify.error).mock.calls.at(-1)![0]).toContain("“Synthetic”的归档操作：未找到原操作回执");
  await act(async () => options.onDismiss?.({id: options.id!} as Parameters<NonNullable<NotificationOptions["onDismiss"]>>[0]));
  const count = vi.mocked(notify.error).mock.calls.length;
  await act(async () => options.actions!.find(action => action.id === "inspect")!.onClick());
  expect(vi.mocked(notify.error).mock.calls.length).toBe(count);
  expect(readConversationArchive(getAppStorage(), conversation.id)?.intent).toEqual(intent);
  expect(setArchived).toHaveBeenCalledTimes(1);
  await act(async () => root.unmount());
  await mount(list, setArchived, undefined, archiveStatus);
  expect(vi.mocked(notify.error).mock.calls.length).toBeGreaterThan(count);
  options = vi.mocked(notify.error).mock.calls.at(-1)![1] as NotificationOptions;
  expect(options.actions?.some(action => action.id === "inspect")).toBe(true);
  expect(readConversationArchive(getAppStorage(), conversation.id)?.intent).toEqual(intent);
});

it("refreshes the row after a conflicting original retry and allows a new archive at the current watermark", async () => {
  let watermark = 14, archived = false;
  const list = vi.fn<AgentCatalogGateway["list"]>(async request => page(request.archived === archived ? [{...conversation, durableCount: watermark, archived}] : []));
  const setArchived = vi.fn<AgentArchiveGateway["setArchived"]>()
    .mockRejectedValueOnce(new Error("lost reply"))
    .mockRejectedValueOnce("conflict")
    .mockImplementation(async request => {archived = request.archived; return archiveProof(request);});
  const archiveStatus = vi.fn<AgentArchiveGateway["archiveStatus"]>(async intent => ({schemaVersion: 1, operationId: intent.operationId,
    conversationId: intent.id, status: {type: "noSqlReceipt", epoch: "22222222-2222-4222-8222-222222222222", sqlFence: "8", closed: false}}));
  const read = vi.fn<AgentMetadataGateway["read"]>(async () => ({...conversation, durableCount: watermark, archived}));
  const {onArchiveChange} = await mount(list, setArchived, read, archiveStatus);
  await act(async () => button("归档对话").click());
  const original = setArchived.mock.calls[0]![0];
  expect(button("归档对话").disabled).toBe(true);
  let options = vi.mocked(notify.error).mock.calls.at(-1)![1] as NotificationOptions;
  await act(async () => options.actions!.find(action => action.id === "inspect")!.onClick());
  options = vi.mocked(notify.error).mock.calls.at(-1)![1] as NotificationOptions;
  watermark = 18;
  await act(async () => options.actions!.find(action => action.id === "retry")!.onClick());
  expect(setArchived.mock.calls[1]![0]).toEqual(original);
  expect(readConversationArchive(getAppStorage(), conversation.id)).toBeNull();
  await vi.waitFor(() => expect(button("归档对话").disabled).toBe(false));
  await act(async () => button("归档对话").click());
  const next = setArchived.mock.calls[2]![0];
  expect(next.operationId).not.toBe(original.operationId);
  expect(next.expectedWatermark).toBe(18);
  expect(onArchiveChange).toHaveBeenCalledWith(conversation.id, true);
  expect(readConversationArchive(getAppStorage(), conversation.id)).toBeNull();
});

it("keeps the list usable but refuses mutation when recovery storage is unavailable", async () => {
  const unavailable = vi.spyOn(appStorage, "getAppStorage").mockImplementation(() => { throw new Error("private storage error"); });
  const setArchived = vi.fn();
  try {
    await mount(vi.fn(async () => page([conversation])), setArchived); expect(button("Synthetic")).toBeDefined();
    await act(async () => button("归档对话").click()); expect(setArchived).not.toHaveBeenCalled();
    expect(notify.error).toHaveBeenLastCalledWith(expect.stringContaining("ARCHIVE_RECOVERY_UNAVAILABLE"));
    expect(vi.mocked(notify.error).mock.calls.flat().join(" ")).not.toContain("private storage error");
  } finally { unavailable.mockRestore(); }
});
