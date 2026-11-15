// @vitest-environment happy-dom
import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AgentCatalogPage, AgentCatalogRequest, AgentConversationSummary } from "@research-assistant/core/agent";
import type { AgentDeletionBatchGateway } from "@research-assistant/data/agent";
import { ConversationArchiveClearDialog } from "./ConversationArchiveClearDialog";
import { BoundConversationList } from "./BoundConversationList";
import { notify } from "../../services/notifications";

vi.mock("../../services/notifications", () => ({ notify: { success: vi.fn(), error: vi.fn(), warning: vi.fn(), dismiss: vi.fn() } }));
let root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.spyOn(HTMLDialogElement.prototype, "showModal").mockImplementation(function (this: HTMLDialogElement) { this.setAttribute("open", ""); });
  const host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {await act(async () => root.unmount()); document.body.replaceChildren(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.clearAllMocks();});
const binding = {kind: "manuscript" as const, resourceId: "manuscript"};
const rows: readonly AgentConversationSummary[] = [1, 2, 3].map(id => ({id: `00000000-0000-4000-8000-00000000000${id}`, title: `Archive ${id}`, archived: true, durableCount: id, createdAt: `time-${id}`, updatedAt: "now"}));
const page = (conversations: readonly AgentConversationSummary[]): AgentCatalogPage => ({schemaVersion: 1, conversations, next: null});
function button(label: string) {
  const found = [...document.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === label || node.getAttribute("aria-label") === label);
  if (!found) throw new Error(`Missing button ${label}`); return found;
}
function fixture() {
  const state = {rows: [...rows]};
  const list = vi.fn(async (_request: AgentCatalogRequest) => page(state.rows));
  const deleteBatch = vi.fn<AgentDeletionBatchGateway["deleteBatch"]>(async requests => {
    const ids = new Set(requests.map(request => request.conversationId));
    state.rows = state.rows.filter(row => !ids.has(row.id));
    return {schemaVersion: 1, results: requests.map(request => ({requestId: request.requestId, conversationId: request.conversationId, status: "deleted" as const, cleanupPending: false}))};
  });
  const onClose = vi.fn(), onDeleted = vi.fn(), onConfirmed = vi.fn();
  return {state, list, deleteBatch, onClose, onDeleted, onConfirmed,
    props: {binding, catalog: {list, rename: vi.fn(), renameStatus: vi.fn()}, deletions: {deleteBatch}, onClose, onDeleted, onConfirmed}};
}

it("confirms once and submits all archived pages in one batch", async () => {
  const f = fixture();
  f.list.mockImplementation(async request => request.before ? page([rows[2]!]) : {...page(rows.slice(0, 2)), next: {createdAt: "time-2", id: rows[1]!.id}});
  await act(async () => root.render(<StrictMode><ConversationArchiveClearDialog {...f.props}/></StrictMode>));
  expect(document.activeElement).toBe(button("取消"));
  expect(document.querySelector("dialog")?.textContent).toContain("3 个已归档对话");
  expect(f.deleteBatch).not.toHaveBeenCalled();
  f.state.rows.push({...rows[0]!, id: "later-archive"});
  await act(async () => button("清空归档").click());
  expect(f.deleteBatch).toHaveBeenCalledOnce();
  const requests = f.deleteBatch.mock.calls[0]![0];
  expect(requests.map(request => request.conversationId)).toEqual(rows.map(row => row.id));
  expect(requests.every(request => request.force && JSON.stringify(request.binding) === JSON.stringify(binding))).toBe(true);
  expect(f.state.rows.map(row => row.id)).toEqual(["later-archive"]);
  expect(f.onConfirmed).toHaveBeenCalledTimes(3);
  expect(f.onDeleted).toHaveBeenCalledExactlyOnceWith(rows.map(row => row.id));
  expect(f.onClose).toHaveBeenCalledOnce();
});

it("prevents duplicate submission and closing while the batch is in progress", async () => {
  const f = fixture(); let finish!: (value: Awaited<ReturnType<AgentDeletionBatchGateway["deleteBatch"]>>) => void;
  f.deleteBatch.mockImplementationOnce(() => new Promise(resolve => {finish = resolve;}));
  await act(async () => root.render(<ConversationArchiveClearDialog {...f.props}/>));
  const confirm = button("清空归档");
  await act(async () => {confirm.click(); confirm.click();});
  await act(async () => document.querySelector("dialog")!.dispatchEvent(new Event("cancel", {cancelable: true})));
  expect(f.deleteBatch).toHaveBeenCalledOnce(); expect(f.onClose).not.toHaveBeenCalled();
  const requests = f.deleteBatch.mock.calls[0]![0];
  await act(async () => finish({schemaVersion: 1, results: requests.map(request => ({requestId: request.requestId, conversationId: request.conversationId, status: "deleted", cleanupPending: false}))}));
  expect(f.onClose).toHaveBeenCalledOnce();
});

it("reports partial failures and only removes confirmed conversations", async () => {
  const f = fixture();
  f.deleteBatch.mockImplementationOnce(async requests => ({schemaVersion: 1, results: requests.map((request, index) => index === 1
    ? {requestId: request.requestId, conversationId: request.conversationId, status: "failed", error: "busy"}
    : {requestId: request.requestId, conversationId: request.conversationId, status: "deleted", cleanupPending: false})}));
  await act(async () => root.render(<ConversationArchiveClearDialog {...f.props}/>));
  await act(async () => button("清空归档").click());
  expect(f.onDeleted).toHaveBeenCalledExactlyOnceWith([rows[0]!.id, rows[2]!.id]);
  expect(f.onConfirmed).toHaveBeenCalledTimes(2); expect(f.onClose).toHaveBeenCalledOnce();
  expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("1 个未完成"));
  expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("busy"));
});

it("distinguishes a deleted conversation whose execution copy still needs cleanup", async () => {
  const f = fixture();
  f.deleteBatch.mockImplementationOnce(async requests => ({schemaVersion: 1, results: requests.map((request, index) => ({
    requestId: request.requestId, conversationId: request.conversationId, status: "deleted", cleanupPending: index === 1,
  }))}));
  await act(async () => root.render(<ConversationArchiveClearDialog {...f.props}/>));
  await act(async () => button("清空归档").click());
  expect(f.onConfirmed).toHaveBeenCalledTimes(3); expect(notify.success).not.toHaveBeenCalled();
  expect(notify.warning).toHaveBeenCalledWith(expect.stringContaining("1 个对话的执行副本仍待清理"));
});

it("reports a local cleanup failure without replaying confirmed deletions", async () => {
  const f = fixture(); const flushCleanup = vi.fn().mockRejectedValue(new Error("private storage details"));
  await act(async () => root.render(<ConversationArchiveClearDialog {...f.props} flushCleanup={flushCleanup}/>));
  await act(async () => button("清空归档").click());
  expect(f.deleteBatch).toHaveBeenCalledOnce(); expect(f.onConfirmed).toHaveBeenCalledTimes(3);
  expect(flushCleanup).toHaveBeenCalledOnce(); expect(f.onDeleted).toHaveBeenCalledExactlyOnceWith(rows.map(row => row.id));
  expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("本地清理未完成"));
  expect(vi.mocked(notify.error).mock.calls.flat().join(" ")).not.toContain("private storage details");
  expect(f.onClose).toHaveBeenCalledOnce();
});

it("does not replay an interrupted batch; another clear requires a fresh confirmation", async () => {
  const f = fixture(); f.deleteBatch.mockRejectedValueOnce("unavailable");
  await act(async () => root.render(<ConversationArchiveClearDialog {...f.props}/>));
  await act(async () => button("清空归档").click());
  expect(f.deleteBatch).toHaveBeenCalledOnce(); expect(f.onClose).toHaveBeenCalledOnce();
  expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("无法确认清空结果"));
  await act(async () => root.render(<div/>));
  await act(async () => root.render(<ConversationArchiveClearDialog {...f.props}/>));
  expect(f.deleteBatch).toHaveBeenCalledOnce();
  await act(async () => button("清空归档").click());
  expect(f.deleteBatch).toHaveBeenCalledTimes(2);
  const previous = f.deleteBatch.mock.calls[0]![0], next = f.deleteBatch.mock.calls[1]![0];
  expect(previous[0]!.requestId).not.toBe(next[0]!.requestId);
});

it("rechecks a failed catalog before requiring a separate confirmation", async () => {
  const f = fixture(); f.list.mockRejectedValueOnce({status: 503, code: "agent_preview_operation_unconfirmed", message: "private provider body"});
  await act(async () => root.render(<ConversationArchiveClearDialog {...f.props}/>));
  const diagnostic = document.querySelector('[role="alert"]')?.textContent;
  expect(diagnostic).toContain("DELETION_BATCH_CATALOG_UNAVAILABLE"); expect(diagnostic).toContain("HTTP 503");
  expect(diagnostic).not.toContain("private");
  await act(async () => button("重试").click());
  expect(f.deleteBatch).not.toHaveBeenCalled(); expect(button("清空归档").disabled).toBe(false);
});

it("ignores UI callbacks on a replaced host while cleaning up confirmed originals", async () => {
  const f = fixture(); let finish!: (value: Awaited<ReturnType<AgentDeletionBatchGateway["deleteBatch"]>>) => void;
  f.deleteBatch.mockImplementationOnce(() => new Promise(resolve => {finish = resolve;}));
  const replacement = vi.fn();
  const view = (identity: string, cleanup: (id: string) => void) => <><ConversationArchiveClearDialog {...f.props} viewIdentity={identity} onConfirmed={cleanup}/><button>新宿主</button></>;
  await act(async () => root.render(view("original", f.onConfirmed)));
  await act(async () => button("清空归档").click());
  const requests = f.deleteBatch.mock.calls[0]![0];
  await act(async () => root.render(view("replacement", replacement))); button("新宿主").focus();
  expect(document.querySelector("dialog")).toBeNull();
  await act(async () => root.render(view("original", f.onConfirmed)));
  expect(document.querySelector("dialog")).toBeNull();
  await act(async () => finish({schemaVersion: 1, results: requests.map(request => ({requestId: request.requestId, conversationId: request.conversationId, status: "deleted", cleanupPending: false}))}));
  expect(f.onConfirmed).toHaveBeenCalledTimes(3); expect(replacement).not.toHaveBeenCalled();
  expect(f.onDeleted).not.toHaveBeenCalled(); expect(f.onClose).not.toHaveBeenCalled(); expect(document.activeElement).toBe(button("新宿主"));
  expect(notify.success).not.toHaveBeenCalled(); expect(notify.warning).not.toHaveBeenCalled(); expect(notify.error).not.toHaveBeenCalled();
});

it.each(["success", "failure"])("ignores a late %s notification after unmount", async outcome => {
  const f = fixture(); let finish!: (value: Awaited<ReturnType<AgentDeletionBatchGateway["deleteBatch"]>>) => void, fail!: (reason: unknown) => void;
  f.deleteBatch.mockImplementationOnce(() => new Promise((resolve, reject) => {finish = resolve; fail = reject;}));
  const flushCleanup = vi.fn(async () => {});
  await act(async () => root.render(<ConversationArchiveClearDialog {...f.props} flushCleanup={flushCleanup}/>));
  await act(async () => button("清空归档").click());
  const requests = f.deleteBatch.mock.calls[0]![0];
  await act(async () => root.render(<button>新宿主</button>)); button("新宿主").focus();
  await act(async () => outcome === "success"
    ? finish({schemaVersion: 1, results: requests.map(request => ({requestId: request.requestId, conversationId: request.conversationId, status: "deleted", cleanupPending: false}))})
    : fail("unavailable"));
  expect(f.onConfirmed).toHaveBeenCalledTimes(outcome === "success" ? 3 : 0);
  expect(flushCleanup).toHaveBeenCalledTimes(outcome === "success" ? 1 : 0);
  expect(f.onDeleted).not.toHaveBeenCalled(); expect(f.onClose).not.toHaveBeenCalled();
  expect(notify.success).not.toHaveBeenCalled(); expect(notify.warning).not.toHaveBeenCalled(); expect(notify.error).not.toHaveBeenCalled();
  expect(document.activeElement).toBe(button("新宿主"));
});

it("explains a rejected batch with its safe reason instead of reporting an unknown outcome", async () => {
  const f = fixture(); f.deleteBatch.mockRejectedValueOnce("busy");
  await act(async () => root.render(<ConversationArchiveClearDialog {...f.props}/>));
  await act(async () => button("清空归档").click());
  expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("仍有运行或恢复任务"));
  expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("code=busy"));
  expect(vi.mocked(notify.error).mock.calls[0]![0]).not.toContain("无法确认清空结果");
  expect(f.deleteBatch).toHaveBeenCalledOnce();
});

it("exposes the toolbar action only in archives and returns focus after cancellation", async () => {
  const f = fixture(); f.list.mockImplementation(async request => page(request.archived ? f.state.rows : []));
  await act(async () => root.render(<BoundConversationList binding={null} selectedId={null} onSelect={vi.fn()}
    catalog={f.props.catalog} batchDeletions={f.props.deletions} deletionConfirmationPreferences={{shouldSkip: () => true, setSkipToday: vi.fn()}}/>));
  expect(document.querySelector('[aria-label="清空归档"]')).toBeNull();
  await act(async () => button("查看归档对话").click());
  const trigger = button("清空归档"); await act(async () => trigger.click());
  expect(f.deleteBatch).not.toHaveBeenCalled(); expect(document.querySelector('dialog[open]')).not.toBeNull();
  await act(async () => button("取消").click());
  expect(document.querySelector("dialog")).toBeNull(); expect(document.activeElement).toBe(trigger);
});

it("scopes internal archives and refuses a mixed-category plan", async () => {
  const f = fixture(); f.state.rows = f.state.rows.map(row => ({...row, internal: true}));
  await act(async () => root.render(<ConversationArchiveClearDialog {...f.props} internal/>));
  expect(f.list.mock.calls.every(([request]) => request.internal === true && request.archived)).toBe(true);
  await act(async () => button("清空归档").click()); expect(f.deleteBatch).toHaveBeenCalledOnce();
  await act(async () => root.render(<div/>)); f.state.rows = [{...rows[0]!, internal: false}];
  await act(async () => root.render(<ConversationArchiveClearDialog {...f.props} internal/>));
  expect(document.querySelector("dialog")?.textContent).not.toContain("1 个已归档对话");
  await act(async () => button("重试").click()); expect(f.deleteBatch).toHaveBeenCalledOnce();
});
