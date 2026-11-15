// @vitest-environment happy-dom
import { createTestConversationForkLock } from "./conversationForkTestLock";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { AgentConversationSummary, AgentForkRequest, AgentForkStatusResponse } from "@research-assistant/core/agent";
import { ConversationForkRecoveries } from "./ConversationForkRecoveries";
import { saveConversationFork } from "./conversationForkPending";
import { acquireConversationForkOwner, resetConversationForkOwnersForTests } from "./conversationForkController";
import { selectAppStorage } from "../../runtime/appStorage";
import { notify } from "../../services/notifications";
vi.mock("../../services/notifications", () => ({ notify: { error: vi.fn(), success: vi.fn() } }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const binding = { kind: "manuscript" as const, resourceId: "synthetic" }, epoch = "00000000-0000-4000-8000-000000000009";
function saved() { return { schemaVersion: 1 as const, intent: { childId: crypto.randomUUID(), parentId: crypto.randomUUID(), throughTurnId: crypto.randomUUID(), binding }, epoch, stage: "committed" as const }; }
afterEach(() => { resetConversationForkOwnersForTests(localStorage); localStorage.clear(); });
function setup() {
  selectAppStorage(localStorage); const container = document.createElement("div"); document.body.append(container); const root = createRoot(container);
  const fork = vi.fn(async () => { throw "must not republish"; });
  const status = vi.fn(async (intent: AgentForkRequest): Promise<AgentForkStatusResponse> => ({ schemaVersion: 1, operationId: intent.childId, conversationId: intent.childId,
    status: { type: "committed", proof: { schemaVersion: 1, operationId: intent.childId, conversationId: intent.childId, epoch, sqlFence: "11", replayed: true, family: "agent.conversation.fork", result: null } } }));
  const read = vi.fn(async ({ id }: { id: string }): Promise<AgentConversationSummary> => ({ id, title: "Current archived child", archived: true, durableCount: 3, createdAt: "2026-10-27T00:00:00.000Z", updatedAt: "2026-10-27T00:00:00.000Z" }));
  const ports = { fork: { fork, status }, metadata: { read }, lock: createTestConversationForkLock() }, open = vi.fn();
  return { container, fork, status, read, open, ports,
    render: async (resourceId = binding.resourceId) => { await act(async () => root.render(<ConversationForkRecoveries binding={{ ...binding, resourceId }} ports={ports} visibleScopes={new Set()} onOpen={open} />)); },
    button: (name: string) => [...container.querySelectorAll<HTMLButtonElement>("button")].find(button => button.getAttribute("aria-label") === name || button.textContent === name)!,
    close: async () => { await act(async () => root.unmount()); container.remove(); } };
}
it("shows a saved fork only after its running operation needs attention", async () => {
  const ui = setup(), original = saved(); let reject!: (cause: unknown) => void;
  ui.status.mockImplementationOnce(() => new Promise((_resolve, fail) => {reject = fail;}));
  try {
    await ui.render();
    const owner = acquireConversationForkOwner(original.intent, ui.ports, localStorage);
    let running!: Promise<unknown>;
    await act(async () => {running = owner.controller.submit();});
    expect(ui.container.textContent).not.toContain("待核对的分支");
    await act(async () => {reject("unavailable"); await running;});
    expect(ui.container.textContent).toContain("待核对的分支（1）");
    owner.release();
  } finally { await ui.close(); }
});
it("restores an original request without the parent projection, inspects before reading, and explicitly opens the current archived child", async () => {
  const original = saved(); saveConversationFork(localStorage, original, true); const ui = setup();
  try {
    await ui.render(); expect(ui.container.textContent).toContain("待核对的分支（1）");
    expect(ui.status).not.toHaveBeenCalled(); expect(ui.read).not.toHaveBeenCalled(); expect(ui.fork).not.toHaveBeenCalled();
    await act(async () => ui.button("核对原分支操作（待核对分支 1）").click());
    expect(ui.status).toHaveBeenCalledExactlyOnceWith(original.intent);
    expect(ui.read).toHaveBeenCalledExactlyOnceWith({ id: original.intent.childId, binding });
    expect(ui.open).not.toHaveBeenCalled();
    await act(async () => ui.button("打开分支（待核对分支 1）").click());
    expect(ui.open).toHaveBeenCalledWith(expect.objectContaining({ id: original.intent.childId, archived: true }));
    expect(ui.container.querySelector("section")).toBeNull(); expect(ui.fork).not.toHaveBeenCalled();
  } finally { await ui.close(); }
});
it("discovers local and other-window changes without RPC or exposing another binding", async () => {
  const ui = setup(); try {
    await ui.render(); expect(ui.container.querySelector("section")).toBeNull(); const original = saved();
    await act(async () => saveConversationFork(localStorage, original, true));
    expect(ui.container.textContent).toContain("待核对的分支（1）");
    await ui.render("other"); expect(ui.container.querySelector("section")).toBeNull();
    expect(ui.status).not.toHaveBeenCalled(); expect(ui.fork).not.toHaveBeenCalled();
    await ui.render(); expect(ui.container.textContent).toContain("待核对的分支（1）");
  } finally { await ui.close(); }
});
it("offers explicit finish for a committed child that was subsequently deleted and removes the recovery row", async () => {
  const original = saved(); saveConversationFork(localStorage, original, true); const ui = setup(); ui.read.mockRejectedValue("notFound");
  try {
    await ui.render(); await act(async () => ui.button("核对原分支操作（待核对分支 1）").click());
    expect(ui.container.textContent).toContain("原分支已提交");
    await act(async () => ui.button("结束已提交分支核对").click());
    expect(ui.container.querySelector("section")).toBeNull(); expect(ui.open).not.toHaveBeenCalled(); expect(ui.fork).not.toHaveBeenCalled();
  } finally { await ui.close(); }
});
it("pages discoveries without dropping later originals or querying them automatically", async () => {
  for (let index = 0; index < 17; index++) saveConversationFork(localStorage, saved(), true);
  const ui = setup(); try {
    await ui.render(); expect(ui.container.querySelectorAll('[role="group"]')).toHaveLength(16);
    await act(async () => ui.button("显示更多分支核对记录").click());
    expect(ui.container.querySelectorAll('[role="group"]')).toHaveLength(17);
    expect(ui.status).not.toHaveBeenCalled(); expect(ui.fork).not.toHaveBeenCalled();
  } finally { await ui.close(); }
});
it("keeps valid discoveries usable and reports a damaged local record by stable code without exposing its body", async () => {
  const original = saved(); saveConversationFork(localStorage, original, true);
  const locator = `research-assistant:conversation-fork:v1:${binding.kind}:${binding.resourceId}:invalid`;
  localStorage.setItem(locator, "PRIVATE_INVALID_BODY");
  const ui = setup(); vi.mocked(notify.error).mockClear();
  try {
    await ui.render(); expect(ui.container.textContent).toContain("待核对的分支（1）");
    expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("FORK_RECOVERY_INVALID"), expect.any(Object));
    expect(JSON.stringify(vi.mocked(notify.error).mock.calls)).not.toContain("PRIVATE_INVALID_BODY");
    expect(localStorage.getItem(locator)).toBe("PRIVATE_INVALID_BODY"); expect(ui.status).not.toHaveBeenCalled();
  } finally { await ui.close(); }
});
it("does not carry discoveries into a replacement data Storage with an identical host binding", async () => {
  const original = saved(); saveConversationFork(localStorage, original, true); sessionStorage.clear();
  const ui = setup(); try {
    await ui.render(); expect(ui.container.textContent).toContain("待核对的分支（1）");
    selectAppStorage(sessionStorage); await ui.render(); expect(ui.container.querySelector("section")).toBeNull();
    selectAppStorage(localStorage); await ui.render(); expect(ui.container.textContent).toContain("待核对的分支（1）");
    expect(ui.status).not.toHaveBeenCalled(); expect(ui.fork).not.toHaveBeenCalled();
  } finally { selectAppStorage(localStorage); await ui.close(); }
});
