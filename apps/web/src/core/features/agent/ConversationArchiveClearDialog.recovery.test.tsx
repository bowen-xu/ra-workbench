// @vitest-environment happy-dom
import { act, StrictMode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AgentConversationSummary, AgentDeletionRequest } from "@research-assistant/core/agent";
import { BoundConversationList } from "./BoundConversationList";
import { saveConversationDeletionBatch, discoverConversationDeletionBatches } from "./conversationDeletionBatchPending";
import { saveConversationDeletion, readConversationDeletion } from "./conversationDeletionPending";
import { createMockAgentRecoveryStorage } from "../../../app/runtime/mock/MockAgentRecoveryStorage";
import { mockWebLocks } from "../../runtime/testing/mockWebLocks";
import { notify, type NotificationOptions } from "../../services/notifications";

vi.mock("../../services/notifications", () => ({notify: {error: vi.fn(), success: vi.fn(), dismiss: vi.fn()}}));
let root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); mockWebLocks();
  const host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  vi.spyOn(HTMLDialogElement.prototype, "showModal").mockImplementation(function (this: HTMLDialogElement) {this.setAttribute("open", "");});
});
afterEach(async () => {await act(async () => root.unmount()); document.body.replaceChildren(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.clearAllMocks();});
const binding = {kind: "manuscript" as const, resourceId: "legacy-host"};
const conversation: AgentConversationSummary = {id: "00000000-0000-4000-8000-000000000001", title: "Archived", archived: true, durableCount: 1, createdAt: "before", updatedAt: "now"};
const request: AgentDeletionRequest = {schemaVersion: 1, requestId: "00000000-0000-4000-8000-000000000002", conversationId: conversation.id,
  binding, expectedTitle: conversation.title, expectedDurableCount: conversation.durableCount, force: true};
function fixture() {
  const storage = createMockAgentRecoveryStorage("legacy-archive-clear");
  saveConversationDeletionBatch(storage, {schemaVersion: 1, batchId: "00000000-0000-4000-8000-000000000003", binding,
    internal: false, epoch: null, requests: [request], phase: "authorized", retained: []});
  saveConversationDeletion(storage, {schemaVersion: 1, intent: request, internal: false, epoch: null, stage: "pending", reason: null, receipt: null}, true);
  const catalog = {list: vi.fn(async () => ({schemaVersion: 1 as const, conversations: [conversation], next: null})), rename: vi.fn(), renameStatus: vi.fn()};
  return {storage, catalog};
}
function button(label: string): HTMLButtonElement {
  const found = [...document.querySelectorAll<HTMLButtonElement>("button")].find(node => node.getAttribute("aria-label") === label);
  if (!found) throw Error(`Missing button ${label}`); return found;
}
it("retires legacy clear records without showing an old recovery entry or blocking a new clear", async () => {
  const f = fixture();
  await act(async () => root.render(<BoundConversationList binding={binding} selectedId={null} onSelect={vi.fn()} catalog={f.catalog}
    revealConversation={conversation} deletionRecoveryStorage={f.storage}/>));
  await vi.waitFor(() => expect(discoverConversationDeletionBatches(f.storage)).toHaveLength(0));
  expect(readConversationDeletion(f.storage, conversation.id)).toBeNull();
  expect(document.querySelector('[aria-label="待核对的清空归档"]')).toBeNull();
  expect(document.querySelector('[aria-label="待核对的删除"]')).toBeNull();
  expect(button("清空归档").disabled).toBe(false);
});
it("blocks conflicting actions while an old owner holds its lock, then retries retirement", async () => {
  const f = fixture(); let blocked = true;
  const lock = {run: async <T,>(_name: string, operation: () => Promise<T>): Promise<T> => {
    if (blocked) throw Error("DELETION_BATCH_LOCK_BUSY"); return operation();
  }};
  await act(async () => root.render(<BoundConversationList binding={binding} selectedId={null} onSelect={vi.fn()} catalog={f.catalog}
    revealConversation={conversation} deletionRecoveryStorage={f.storage} deletionRecoveryLock={lock}/>));
  await vi.waitFor(() => expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("DELETION_BATCH_LOCK_BUSY"), expect.any(Object)));
  expect(button("清空归档").disabled).toBe(true);
  expect(discoverConversationDeletionBatches(f.storage)).toHaveLength(1);
  expect(document.querySelector('[aria-label="待核对的清空归档"]')).toBeNull();
  blocked = false;
  const options = vi.mocked(notify.error).mock.calls.at(-1)![1] as NotificationOptions;
  await act(async () => options.actions![0]!.onClick());
  expect(discoverConversationDeletionBatches(f.storage)).toHaveLength(0);
  expect(button("清空归档").disabled).toBe(false);
  expect(notify.dismiss).toHaveBeenCalledWith("legacy-conversation-clear");
});

it("keeps multiple lists disabled until their shared legacy retirement completes", async () => {
  const f = fixture(); let release!: () => void;
  const gate = new Promise<void>(resolve => {release = resolve;});
  const lock = {run: async <T,>(name: string, operation: () => Promise<T>): Promise<T> => {
    if (name.includes("conversation-deletion-batch-lock")) await gate; return operation();
  }};
  const list = () => <BoundConversationList binding={binding} selectedId={null} onSelect={vi.fn()} catalog={f.catalog}
    revealConversation={conversation} deletionRecoveryStorage={f.storage} deletionRecoveryLock={lock}/>;
  await act(async () => root.render(<StrictMode>{list()}{list()}</StrictMode>));
  const clearButtons = () => [...document.querySelectorAll<HTMLButtonElement>('[aria-label="清空归档"]')];
  expect(clearButtons()).toHaveLength(2); expect(clearButtons().every(item => item.disabled)).toBe(true);
  expect(notify.error).not.toHaveBeenCalled();
  await act(async () => release());
  await vi.waitFor(() => expect(clearButtons().every(item => !item.disabled)).toBe(true));
  expect(discoverConversationDeletionBatches(f.storage)).toHaveLength(0);
  expect(notify.error).not.toHaveBeenCalled();
});
