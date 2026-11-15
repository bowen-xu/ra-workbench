// @vitest-environment happy-dom
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { NoteRewriteUndo } from "./NoteRewriteUndo";
const f = vi.hoisted(() => ({status: vi.fn(), error: vi.fn()}));
vi.mock("../../../services/agentGateway", () => ({agentNoteUndoGateway: {status: f.status}}));
vi.mock("../../../services/notifications", () => ({notify: {error: f.error}}));
const application = {schemaVersion: 1 as const, conversationId: "a1111111-1111-4111-8111-111111111111", pageId: "b1111111-1111-4111-8111-111111111111",
  turnId: "c1111111-1111-4111-8111-111111111111", itemId: "d1111111-1111-4111-8111-111111111111", applicationId: "e1111111-1111-4111-8111-111111111111",
  proposalId: `note-rewrite:${"a".repeat(64)}`, beforeRevision: 3, appliedRevision: 4, beforeHash: "a".repeat(64), afterHash: "b".repeat(64), createdAt: "2026-09-11T01:02:03.000Z"};
let host: HTMLDivElement, root: ReturnType<typeof createRoot>;
beforeEach(() => { vi.resetAllMocks(); vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); host = document.createElement("div"); document.body.append(host); root = createRoot(host); f.status.mockResolvedValue(null); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
async function render(onUndo = vi.fn(async () => {}), revision = 4, blocked = false) {
  await act(async () => root.render(<StrictMode><NoteRewriteUndo application={application} currentRevision={revision} isSaved blocked={blocked} onUndo={onUndo}/></StrictMode>));
}
it("loads history without writing and prevents a second undo after a durable result", async () => {
  f.status.mockResolvedValue({restoredRevision: 5, createdAt: application.createdAt}); const undo = vi.fn(async () => {});
  await render(undo); expect(host.textContent).toContain("已撤销至版本 5"); expect(host.querySelector("button")).toBeNull(); expect(undo).not.toHaveBeenCalled();
});
it("serializes clicks and refreshes the durable result after the workspace finishes", async () => {
  let done!: () => void; const undo = vi.fn(() => new Promise<void>(resolve => {done = resolve;})); await render(undo);
  await act(async () => {host.querySelector("button")!.click(); host.querySelector("button")!.click();}); expect(undo).toHaveBeenCalledOnce();
  f.status.mockResolvedValue({restoredRevision: 5, createdAt: application.createdAt});
  await act(async () => done()); expect(host.textContent).toContain("已撤销至版本 5");
});
it("blocks stale or unresolved operations and preserves an explicit read retry", async () => {
  const undo = vi.fn(async () => {}); await render(undo, 5); expect(host.querySelector("button")!.disabled).toBe(true);
  await render(undo, 4, true); expect(host.querySelector("button")!.disabled).toBe(true);
  f.status.mockRejectedValue(new Error("offline")); await render(undo, 4, false);
  expect(host.textContent).toContain("重试读取撤销状态"); expect(undo).not.toHaveBeenCalled();
  f.status.mockResolvedValue(null); await act(async () => host.querySelector("button")!.click());
  expect(host.textContent).toContain("撤销这次改写");
});
it("ignores pending UI replies after hiding while the workspace retains ownership", async () => {
  let done!: () => void; const undo = vi.fn(() => new Promise<void>(resolve => {done = resolve;})); await render(undo);
  await act(async () => host.querySelector("button")!.click()); await act(async () => root.render(null));
  await act(async () => done()); expect(host.textContent).toBe(""); expect(undo).toHaveBeenCalledOnce();
});
