// @vitest-environment happy-dom
import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { BoundConversationList } from "./BoundConversationList";
import { ConversationDeleteDialog } from "./ConversationDeleteDialog";
import { createConversationDeletePreviewData, conversationDeletePreviewBinding as binding } from "../../../app/previews/agent/conversationDeletePreviewData";
import { readConversationDeletion } from "./conversationDeletionPending";
import { resetConversationDeletionOwnersForTests } from "./conversationDeletionController";
import { mockWebLocks } from "../../runtime/testing/mockWebLocks";
import { notify } from "../../services/notifications";
import type { AgentConversationBinding } from "@research-assistant/core/agent";

vi.mock("../../services/notifications", () => ({notify: {error: vi.fn(), success: vi.fn(), dismiss: vi.fn()}}));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root, host: HTMLDivElement;
const live: ReturnType<typeof createConversationDeletePreviewData>[] = [];
beforeEach(() => {
  mockWebLocks(); host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  vi.spyOn(HTMLDialogElement.prototype, "showModal").mockImplementation(function (this: HTMLDialogElement) {this.setAttribute("open", "");});
});
afterEach(async () => {
  await act(async () => root.unmount()); document.body.replaceChildren();
  for (const data of live.splice(0)) resetConversationDeletionOwnersForTests(data.recoveryStorage, data.deletions);
  vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.clearAllMocks();
});
function fixture(scenario: Parameters<typeof createConversationDeletePreviewData>[0]) {
  const data = createConversationDeletePreviewData(scenario, 0); live.push(data); return data;
}
function button(text: string): HTMLButtonElement {
  const found = [...document.querySelectorAll<HTMLButtonElement>("button")].find(node => node.textContent === text || node.getAttribute("aria-label") === text);
  if (!found) throw Error(`Missing button ${text}`); return found;
}
async function render(data: ReturnType<typeof fixture>, viewBinding: AgentConversationBinding | null = binding) {
  await act(async () => root.render(<StrictMode><BoundConversationList binding={viewBinding} selectedId={data.firstConversation.id}
    revealConversation={data.firstConversation} onSelect={vi.fn()} catalog={data.catalog} archives={data.archives} metadata={data.metadata}
    search={data.search} deletions={data.deletions} exports={data.exports} deletionRecoveryStorage={data.recoveryStorage}
    deletionConfirmationPreferences={data.deletionConfirmationPreferences} forceDeletionConfirmationPreferences={data.forceDeletionConfirmationPreferences} /></StrictMode>));
}
it("reopens a missing row from original saved intent, ignores skip preference on recovery and never deletes twice", async () => {
  const data = fixture("committedAckLost"), status = vi.spyOn(data.deletions, "status"); await render(data);
  expect(status).not.toHaveBeenCalled();
  await act(async () => button("永久删除对话").click());
  await act(async () => {document.querySelector<HTMLInputElement>(".conversation-delete-skip input")!.click(); button("确认").click();});
  const original = readConversationDeletion(data.recoveryStorage, data.firstConversation.id)!;
  expect(original.stage).toBe("unknown"); expect(data.attempts).toHaveLength(1); expect(status).toHaveBeenCalledOnce();
  await act(async () => root.unmount()); resetConversationDeletionOwnersForTests(data.recoveryStorage, data.deletions);
  root = createRoot(host); await render(data);
  expect(document.querySelectorAll(".conversation-list-row")).toHaveLength(2);
  expect(document.querySelector("dialog")).toBeNull(); expect(status).toHaveBeenCalledOnce();
  await act(async () => button(`核对“${original.intent.expectedTitle}”`).click());
  expect(document.querySelector<HTMLInputElement>(".conversation-delete-skip input")!.checked).toBe(true);
  expect(data.attempts).toHaveLength(1); expect(status).toHaveBeenCalledOnce();
  await act(async () => button("核对原删除").click());
  expect(status).toHaveBeenLastCalledWith(original.intent); expect(data.attempts).toHaveLength(1);
  expect(readConversationDeletion(data.recoveryStorage, data.firstConversation.id)?.stage).toBe("committed");
  expect(document.querySelector('dialog [role="alert"]')).toBeNull();
  expect(document.querySelector('dialog [role="status"]')?.textContent).toContain("原删除已确认");
  await act(async () => button("完成本机收尾").click());
  expect(document.querySelector("dialog")).toBeNull(); expect(document.querySelector('[aria-label="待核对的删除"]')).toBeNull();
  expect(readConversationDeletion(data.recoveryStorage, data.firstConversation.id)).toBeNull(); expect(data.attempts).toHaveLength(1);
  expect(notify.success).toHaveBeenCalledExactlyOnceWith(expect.stringContaining(original.intent.expectedTitle));
});
it("does not show the active deletion as a recovery behind its dialog", async () => {
  const data = fixture("committedAckLost");
  let reject!: (cause: unknown) => void;
  vi.spyOn(data.deletions, "delete").mockImplementationOnce(() => new Promise((_resolve, fail) => {reject = fail;}));
  await render(data); await act(async () => button("永久删除对话").click());
  await act(async () => button("确认").click());
  expect(readConversationDeletion(data.recoveryStorage, data.firstConversation.id)).not.toBeNull();
  expect(document.querySelector('[aria-label="待核对的删除"]')).toBeNull();
  await act(async () => reject("unavailable"));
  await act(async () => button("取消").click());
  expect(document.querySelector('[aria-label="待核对的删除"]')).not.toBeNull();
});
it("repairs initial local storage failure before any status or deletion and keeps one request identity", async () => {
  const data = fixture("recoverySaveFailure"), status = vi.spyOn(data.deletions, "status"); await render(data);
  await act(async () => button("永久删除对话").click()); await act(async () => button("确认").click());
  expect(status).not.toHaveBeenCalled(); expect(data.attempts).toHaveLength(0);
  const diagnostic = vi.mocked(notify.error).mock.calls.at(-1)![0], operation = /operation=([\da-f-]+)/u.exec(diagnostic)?.[1];
  expect(operation).toBeDefined(); expect(diagnostic).toContain("DELETION_RECOVERY_UNAVAILABLE");
  await act(async () => button("重试本机恢复记录").click());
  expect(status).toHaveBeenCalledOnce(); expect(data.attempts).toHaveLength(0);
  expect(readConversationDeletion(data.recoveryStorage, data.firstConversation.id)?.intent.requestId).toBe(operation);
  await act(async () => button("重试原删除").click()); expect(data.attempts).toHaveLength(1);
  expect(data.attempts[0]!.requestId).toBe(operation); expect(document.querySelector("dialog")).toBeNull();
});
it("does not adopt or cancel another host's original deletion through a stale dialog", async () => {
  const data = fixture("busy"), status = vi.spyOn(data.deletions, "status"); await render(data);
  await act(async () => button("永久删除对话").click()); await act(async () => button("确认").click());
  const saved = readConversationDeletion(data.recoveryStorage, data.firstConversation.id)!;
  await act(async () => root.unmount()); resetConversationDeletionOwnersForTests(data.recoveryStorage, data.deletions);
  root = createRoot(host); const onClose = vi.fn();
  await act(async () => root.render(<ConversationDeleteDialog conversation={data.firstConversation} binding={{...binding, resourceId: "different"}}
    gateway={data.deletions} recoveryStorage={data.recoveryStorage} recoveryOnly onClose={onClose} onConfirmed={vi.fn()} onDeleted={vi.fn()} />));
  expect(document.querySelector('[role="alert"]')?.textContent).toContain("DELETION_RECOVERY_SCOPE_CONFLICT");
  expect(button("重试").disabled).toBe(true); expect(status).toHaveBeenCalledOnce(); expect(data.attempts).toHaveLength(1);
  await act(async () => button("取消").click()); expect(onClose).toHaveBeenCalledOnce();
  expect(readConversationDeletion(data.recoveryStorage, data.firstConversation.id)).toEqual(saved);
});

it("recovers a bound request from the global catalog with missing conversation metadata and preserves the original binding", async () => {
  const data = fixture("committedAckLost"), status = vi.spyOn(data.deletions, "status"); await render(data);
  await act(async () => button("永久删除对话").click()); await act(async () => button("确认").click());
  const original = readConversationDeletion(data.recoveryStorage, data.firstConversation.id)!;
  await act(async () => root.unmount()); resetConversationDeletionOwnersForTests(data.recoveryStorage, data.deletions);
  const metadata = vi.spyOn(data.metadata, "read").mockRejectedValue("notFound");
  root = createRoot(host); await render(data, null);
  expect(document.querySelector('[aria-label="待核对的删除"] small')?.textContent).toBe("原稿件");
  expect(status).toHaveBeenCalledOnce(); expect(data.attempts).toHaveLength(1);
  await act(async () => button(`核对“${original.intent.expectedTitle}”`).click());
  await act(async () => button("核对原删除").click());
  expect(status).toHaveBeenLastCalledWith(original.intent);
  expect(status.mock.calls.at(-1)![0].binding).toEqual(binding);
  await act(async () => button("完成本机收尾").click());
  // Catalog synchronization may still look up the selected conversation. Its
  // missing metadata must not prevent proof recovery or rebind the request.
  expect(data.attempts).toHaveLength(1); expect(metadata).toHaveBeenCalledWith({id: data.firstConversation.id});
  expect(readConversationDeletion(data.recoveryStorage, data.firstConversation.id)).toBeNull();
});

it("retains original cleanup and ignores late view callbacks when the global recovery view changes", async () => {
  const data = fixture("success"), cleanup = vi.fn(), replacementCleanup = vi.fn(), deleted = vi.fn();
  let release!: () => void, entered!: () => void;
  const active = new Promise<void>(resolve => {entered = resolve;}), gate = new Promise<void>(resolve => {release = resolve;});
  const remove = data.deletions.delete;
  vi.spyOn(data.deletions, "delete").mockImplementation(async (...args) => {entered(); await gate; return remove(...args);});
  const view = (identity: string, onConfirmed: () => void) => <><ConversationDeleteDialog conversation={data.firstConversation} binding={binding}
    gateway={data.deletions} recoveryStorage={data.recoveryStorage} onClose={vi.fn()} onDeleted={deleted} onConfirmed={onConfirmed}
    viewIdentity={identity} /><button>新宿主</button></>;
  await act(async () => root.render(view("original", cleanup)));
  await act(async () => button("确认").click()); await active;
  await act(async () => root.render(view("replacement", replacementCleanup))); button("新宿主").focus();
  expect(document.querySelector("dialog")).toBeNull(); await act(async () => release());
  expect(cleanup).toHaveBeenCalledExactlyOnceWith(data.firstConversation.id); expect(replacementCleanup).not.toHaveBeenCalled();
  expect(deleted).not.toHaveBeenCalled(); expect(document.activeElement).toBe(button("新宿主"));
});
