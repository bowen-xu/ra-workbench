/** @vitest-environment happy-dom */
import { createTestConversationForkLock } from "./conversationForkTestLock";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { AgentConversationProjection, AgentConversationSummary, AgentForkRequest, AgentForkResponse, AgentForkStatusResponse, AgentTurnProjection } from "@research-assistant/core/agent";
import { useConversationForkActions } from "./useConversationForkActions";
import { resetConversationForkOwnersForTests } from "./conversationForkController";
import { selectAppStorage } from "../../runtime/appStorage";
import { notify } from "../../services/notifications";
vi.mock("../../services/notifications", () => ({ notify: { error: vi.fn(), success: vi.fn() } }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const parentId = "00000000-0000-4000-8000-000000000001", done = "00000000-0000-4000-8000-000000000002";
const empty = "00000000-0000-4000-8000-000000000003", active = "00000000-0000-4000-8000-000000000004", epoch = "00000000-0000-4000-8000-000000000009";
const turn: AgentTurnProjection = { contextSnapshotId: "snapshot", model: { providerId: "test", modelId: "test", revision: null }, phase: "preparing", outcome: "completed", itemOrder: ["user", "answer"], items: new Map(), attemptOrder: [], attempts: new Map(), inputTokens: null, outputTokens: null };
const projection: AgentConversationProjection = { conversationId: parentId, seq: 5, activeTurn: active, turnOrder: [done, empty, active],
  turns: new Map([[done, turn], [empty, { ...turn, outcome: "failed", itemOrder: [] }], [active, { ...turn, outcome: null }]]) };
const summary = (id: string, archived = false): AgentConversationSummary => ({ id, title: "Later name", archived, durableCount: 5, createdAt: "2026-10-27T00:00:00.000Z", updatedAt: "2026-10-27T00:00:00.000Z" });
const response = (request: AgentForkRequest): AgentForkResponse => ({ schemaVersion: 1, conversation: summary(request.childId), parentId: request.parentId, throughTurnId: request.throughTurnId, parentEndSeq: 5 });
function deferred<T>() { let resolve!: (value: T) => void; let reject!: (cause: unknown) => void; const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
afterEach(() => { resetConversationForkOwnersForTests(localStorage); localStorage.clear(); vi.restoreAllMocks(); });
function setup() {
  selectAppStorage(localStorage); vi.mocked(notify.error).mockClear(); vi.mocked(notify.success).mockClear();
  const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
  let committed = false, archived = false;
  const fork = vi.fn(async (request: AgentForkRequest) => { committed = true; return response(request); });
  const status = vi.fn(async (request: AgentForkRequest): Promise<AgentForkStatusResponse> => ({ schemaVersion: 1, operationId: request.childId, conversationId: request.childId,
    status: committed ? { type: "committed", proof: { schemaVersion: 1, operationId: request.childId, conversationId: request.childId, family: "agent.conversation.fork", epoch, sqlFence: "7", replayed: true, result: null } }
      : { type: "noSqlReceipt", epoch, sqlFence: "0", closed: false } }));
  const metadata = { read: vi.fn(async ({ id }: { id: string }) => summary(id, archived)) }, gateway = { fork, status }, lock = createTestConversationForkLock(), open = vi.fn();
  function Probe({ host, visible, disabled, gone }: { host: string; visible: boolean; disabled: boolean; gone: boolean }) {
    const actions = useConversationForkActions({ parentId: gone ? null : parentId, binding: { kind: "manuscript", resourceId: host }, projection: gone ? null : projection, disabled, onOpen: open, gateway, metadata, lock });
    return visible ? <><textarea defaultValue="Unsent draft" />{projection.turnOrder.map(id => <article key={id} data-turn={id}>{actions.messageActions(`${id}/answer`)}</article>)}{actions.historyActions}</> : null;
  }
  return { container, fork, status, metadata, open, commit: () => { committed = true; }, archive: () => { archived = true; },
    render: async (host = "one", visible = true, disabled = false, viewKey = host, gone = false) => { await act(async () => root.render(<Probe key={viewKey} host={host} visible={visible} disabled={disabled} gone={gone} />)); },
    button: () => container.querySelector<HTMLButtonElement>(`[data-turn="${done}"] button`)!,
    retry: () => [...container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "重试原分支请求")!,
    close: async () => { await act(async () => root.unmount()); container.remove(); } };
}
it("offers terminal and empty-turn boundaries; metadata gating never starts an operation", async () => {
  const ui = setup(); try {
    await ui.render("one", true, true); expect(ui.container.querySelectorAll("button")).toHaveLength(2);
    expect(ui.container.querySelector(`[data-turn="${active}"] button`)).toBeNull();
    expect(ui.container.textContent).toContain("第 2 轮已结束，未生成消息");
    await act(async () => ui.button().click()); expect(ui.fork).not.toHaveBeenCalled(); expect(ui.status).not.toHaveBeenCalled();
    await ui.render(); expect(ui.button().disabled).toBe(false);
  } finally { await ui.close(); }
});
it.each([false, true])("preserves draft and requires explicit open using current metadata (archived=%s)", async archived => {
  const ui = setup(), pending = deferred<AgentForkResponse>(); if (archived) ui.archive();
  ui.fork.mockImplementationOnce(async request => { const value = await pending.promise; ui.commit(); return { ...value, conversation: summary(request.childId) }; });
  try {
    await ui.render(); const input = ui.container.querySelector("textarea");
    await act(async () => { ui.button().click(); ui.button().click(); });
    expect(ui.fork).toHaveBeenCalledOnce(); expect(ui.button().disabled).toBe(true);
    await act(async () => pending.resolve(response(ui.fork.mock.calls[0]![0])));
    expect(ui.open).not.toHaveBeenCalled(); expect(ui.button().textContent).toContain("打开分支");
    expect(ui.container.querySelector("textarea")).toBe(input); expect(input?.value).toBe("Unsent draft");
    await act(async () => ui.button().click()); expect(ui.open).toHaveBeenCalledWith(summary(ui.fork.mock.calls[0]![0].childId, archived));
    expect(localStorage.length).toBe(0);
  } finally { await ui.close(); }
});
it("restores a lost-ACK request after remount and checks before reading, without another mutation", async () => {
  const ui = setup(); ui.fork.mockImplementationOnce(async () => { ui.commit(); throw "lost_ack"; });
  try {
    await ui.render(); await act(async () => ui.button().click()); const original = ui.fork.mock.calls[0]![0];
    expect(ui.button().textContent).toContain("核对原分支操作");
    await ui.render("one", true, false, "cold-view"); expect(ui.status).toHaveBeenCalledOnce();
    expect(ui.metadata.read).not.toHaveBeenCalled(); await act(async () => ui.button().click());
    expect(ui.fork).toHaveBeenCalledOnce(); expect(ui.status.mock.calls[1]![0]).toEqual(original);
    expect(ui.metadata.read).toHaveBeenCalledWith({ id: original.childId, binding: original.binding });
    expect(ui.button().textContent).toContain("打开分支"); expect(ui.open).not.toHaveBeenCalled();
  } finally { await ui.close(); }
});
it("offers an explicit same-intent retry only after readonly no-receipt inspection", async () => {
  const ui = setup(); ui.fork.mockRejectedValueOnce("before_commit_unknown");
  try {
    await ui.render(); await act(async () => ui.button().click()); expect(ui.retry()).toBeUndefined();
    await act(async () => ui.button().click()); expect(ui.fork).toHaveBeenCalledOnce();
    expect(ui.container.textContent).toContain("不表示从未创建"); await act(async () => ui.retry().click());
    expect(ui.fork.mock.calls[1]![0]).toEqual(ui.fork.mock.calls[0]![0]); expect(ui.button().textContent).toContain("打开分支");
  } finally { await ui.close(); }
});
it("does not notify, select, or replace the new host with a late result", async () => {
  const ui = setup(), pending = deferred<AgentForkResponse>(); ui.fork.mockReturnValueOnce(pending.promise);
  try {
    await ui.render(); await act(async () => ui.button().click()); await ui.render("two");
    await act(async () => pending.reject("lost_ack")); expect(notify.error).not.toHaveBeenCalled(); expect(ui.open).not.toHaveBeenCalled();
    expect(ui.button().textContent).toContain("从此轮创建分支");
    await ui.render(); expect(ui.button().textContent).toContain("核对原分支操作");
  } finally { await ui.close(); }
});
it("shows safe original diagnostics without private body or a foreign operation's storage cause", async () => {
  const ui = setup(); ui.fork.mockImplementationOnce(async request => { throw { code: "storage", reason: { code: "DATA_SQL_COMMIT_UNKNOWN",
    reason: { operationId: request.childId, phase: "commit", safeCause: "sql_commit_outcome_unknown", sqliteCode: "ConstraintViolation", sqliteExtendedCode: 787, rawSql: "PRIVATE_SYNTHETIC_BODY" } } }; });
  try {
    await ui.render(); await act(async () => ui.button().click()); const message = vi.mocked(notify.error).mock.calls[0]![0];
    expect(message).toContain("DATA_SQL_COMMIT_UNKNOWN"); expect(message).toContain("787"); expect(message).toContain(ui.fork.mock.calls[0]![0].childId); expect(message).not.toContain("PRIVATE_SYNTHETIC_BODY");
    ui.fork.mockRejectedValueOnce({ code: "storage", reason: { code: "DATA_SQL_COMMIT_UNKNOWN", reason: { operationId: crypto.randomUUID(), phase: "commit", safeCause: "sql_commit_outcome_unknown" } } });
    await act(async () => ui.button().click()); await act(async () => ui.retry().click());
    expect(vi.mocked(notify.error).mock.calls.at(-1)![0]).not.toContain("DATA_SQL_COMMIT_UNKNOWN");
  } finally { await ui.close(); }
});
it.each([true, false])("keeps original fork recovery available when history actions are disabled, with missing history=%s", async gone => {
  const ui = setup(); ui.fork.mockImplementationOnce(async () => { ui.commit(); throw "lost_ack"; });
  try {
    await ui.render(); await act(async () => ui.button().click()); const original = ui.fork.mock.calls[0]![0];
    await ui.render("one", true, true, "one", gone);
    const recovery = ui.container.querySelector('section[aria-label="待核对的分支"]');
    expect(recovery).not.toBeNull(); expect(ui.status).toHaveBeenCalledOnce();
    const inspect = recovery!.querySelector<HTMLButtonElement>('button[aria-label="核对原分支操作（待核对分支 1）"]')!;
    expect(inspect.disabled).toBe(false); await act(async () => inspect.click());
    expect(ui.status.mock.calls[1]![0]).toEqual(original); expect(ui.fork).toHaveBeenCalledOnce();
    const open = recovery!.querySelector<HTMLButtonElement>('button[aria-label="打开分支（待核对分支 1）"]')!;
    await act(async () => open.click()); expect(ui.open).toHaveBeenCalledWith(summary(original.childId));
    expect(ui.container.querySelector('section[aria-label="待核对的分支"]')).toBeNull();
  } finally { await ui.close(); }
});
it("reports the original data epoch rejection with actionable context and original storage diagnostics", async () => {
  const ui = setup(); ui.fork.mockImplementationOnce(async request => { throw {code:"storage",reason:{code:"DATA_SQL_OPERATION_CONFLICT",
    reason:{operationId:request.childId,phase:"fork-epoch",safeCause:"data_epoch_changed"}}}; });
  try {
    await ui.render(); await act(async () => ui.button().click());
    expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("请回到原数据环境核对"));
    const message = vi.mocked(notify.error).mock.calls[0]![0];
    expect(message).toContain("DATA_SQL_OPERATION_CONFLICT"); expect(message).toContain(ui.fork.mock.calls[0]![0].childId);
    expect(ui.button().textContent).toContain("核对原分支操作"); expect(ui.open).not.toHaveBeenCalled();
  } finally { await ui.close(); }
});
