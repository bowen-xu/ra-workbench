/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AgentChildCreationStatusResponse } from "@research-assistant/core/agent";
import { AgentChildCreationRecovery, type ChildCreationRecoveryIdentity } from "./AgentChildCreationRecovery";

const identity: ChildCreationRecoveryIdentity = { conversationId: "11111111-1111-4111-8111-111111111111",
  turnId: "22222222-2222-4222-8222-222222222222", callId: "33333333-3333-4333-8333-333333333333",
  childId: "44444444-4444-4444-8444-444444444444", operationId: "55555555-5555-4555-8555-555555555555", watermark: 10 };
const response = (): AgentChildCreationStatusResponse => ({schemaVersion: 1, conversationId: identity.conversationId,
  turnId: identity.turnId, callId: identity.callId, watermark: 10, creation: {schemaVersion: 1,
    conversationId: identity.childId, operationId: identity.operationId,
    status: {type: "noSqlReceipt", epoch: "66666666-6666-4666-8666-666666666666", sqlFence: "0", closed: false}}});
const createdResponse = (): AgentChildCreationStatusResponse => ({...response(), creation: {...response().creation,
  status: {type: "committed", proof: {schemaVersion: 1, operationId: identity.operationId, conversationId: identity.childId,
    family: "agent.conversation.create_bound", epoch: "66666666-6666-4666-8666-666666666666", sqlFence: "0", replayed: false, result: null}}}});
let host: HTMLDivElement;
let root: ReturnType<typeof createRoot>;
const writeText = vi.fn(async (_value: string) => undefined);
beforeEach(() => { (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
  writeText.mockClear(); Object.defineProperty(navigator, "clipboard", {configurable: true, value: {writeText}});
  host = document.createElement("div"); document.body.append(host); root = createRoot(host); });
afterEach(async () => { await act(async () => root.unmount()); host.remove(); });
async function click() { await act(async () => [...host.querySelectorAll("button")].find(button => button.textContent === "核对子任务创建")!.click()); }
async function diagnostic() { await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="复制诊断信息"]')!.click()); return writeText.mock.lastCall?.[0] ?? ""; }

it("inspects the original call without treating absence of a receipt as permission to retry", async () => {
  const gateway = {read: vi.fn(async () => response())};
  await act(async () => root.render(<AgentChildCreationRecovery identity={identity} gateway={gateway} />));
  await click();
  expect(gateway.read).toHaveBeenCalledExactlyOnceWith({schemaVersion: 1, conversationId: identity.conversationId, turnId: identity.turnId, callId: identity.callId});
  expect(host.textContent).toContain("未找到原创建回执");
  expect(host.textContent).not.toContain("子任务的原提交记录");
  expect(host.textContent).toContain("对话仍需恢复");
  expect(host.querySelectorAll("button")).toHaveLength(2);
});

it("shows the original cleanup receipt without releasing recovery", async () => {
  const gateway = {read: vi.fn(async () => ({...response(), cleanup: {schemaVersion: 1 as const,
    conversationId: identity.childId, operationId: "88888888-8888-4888-8888-888888888888",
    status: {type: "noSqlReceipt" as const, epoch: "66666666-6666-4666-8666-666666666666", sqlFence: "0", closed: false}}}))};
  await act(async () => root.render(<AgentChildCreationRecovery identity={identity} gateway={gateway} />));
  await click();
  expect(host.textContent).toContain("未找到原清理归档回执");
  expect(host.textContent).toContain("对话仍需恢复");
});

it.each([true, false])("shows the retained submission snapshot without permitting recovery (%s)", async present => {
  const submissionRequestId = "77777777-7777-4777-8777-777777777777";
  const gateway = {read: vi.fn(async () => ({...createdResponse(), submission: {requestId: submissionRequestId,
    epoch: "66666666-6666-4666-8666-666666666666", record: present ? {turnId: identity.turnId, contextSnapshotId: identity.childId} : null}}))};
  await act(async () => root.render(<AgentChildCreationRecovery identity={{...identity, submissionRequestId}} gateway={gateway} />));
  await click();
  expect(host.textContent).toContain(present ? "当前保留子任务的原提交记录" : "当前未找到子任务的原提交记录");
  expect(host.textContent).toContain("对话仍需恢复");
  expect(host.querySelectorAll("button")).toHaveLength(2);
});

it("rejects a submission snapshot for a different original request", async () => {
  const gateway = {read: vi.fn(async () => ({...createdResponse(), submission: {requestId: identity.callId,
    epoch: "66666666-6666-4666-8666-666666666666", record: null}}))};
  await act(async () => root.render(<AgentChildCreationRecovery identity={{...identity, submissionRequestId: identity.turnId}} gateway={gateway} />));
  await click();
  expect(await diagnostic()).toContain("CHILD_CREATION_IDENTITY_MISMATCH");
});

it.each([
  [{type: "committed", proof: {schemaVersion: 1, operationId: "88888888-8888-4888-8888-888888888888", conversationId: identity.childId,
    family: "agent.conversation.archive", epoch: "66666666-6666-4666-8666-666666666666", sqlFence: "1", replayed: false, result: null}}, "已找到原清理归档回执"],
  [{type: "unconfirmed", epoch: "66666666-6666-4666-8666-666666666666", sqlFence: "0", blocked: true, closed: false, reason: "owner_blocked"}, "清理归档回执仍无法确认"],
] as const)("shows cleanup status %s without continuing", async (status, message) => {
  const gateway = {read: vi.fn(async () => ({...response(), cleanup: {schemaVersion: 1 as const,
    conversationId: identity.childId, operationId: "88888888-8888-4888-8888-888888888888", status}}))};
  await act(async () => root.render(<AgentChildCreationRecovery identity={identity} gateway={gateway} />));
  await click();
  expect(host.textContent).toContain(message);
  expect(host.textContent).toContain("对话仍需恢复");
});

it.each([
  [{type: "committed", proof: {schemaVersion: 1, operationId: identity.operationId, conversationId: identity.childId,
    family: "agent.conversation.create_bound", epoch: "66666666-6666-4666-8666-666666666666", sqlFence: "1", replayed: false, result: null}}, "已找到原创建回执"],
  [{type: "unconfirmed", epoch: "66666666-6666-4666-8666-666666666666", sqlFence: "0", blocked: true, closed: false, reason: "owner_blocked"}, "创建回执仍无法确认"],
] as const)("shows the creation stage without releasing recovery for %s", async (status, label) => {
  const gateway = {read: vi.fn(async () => ({...response(), creation: {...response().creation, status}}))};
  await act(async () => root.render(<AgentChildCreationRecovery identity={identity} gateway={gateway} />));
  await click();
  expect(host.textContent).toContain(label);
  expect(host.textContent).toContain("对话仍需恢复");
});

it("rejects a mismatched or stale response and preserves a safe error code", async () => {
  const gateway = {read: vi.fn().mockResolvedValueOnce({...response(), watermark: 9})
    .mockResolvedValueOnce({...response(), cleanup: {schemaVersion: 1, conversationId: identity.conversationId,
      operationId: identity.operationId, status: {type: "noSqlReceipt", epoch: "66666666-6666-4666-8666-666666666666", sqlFence: "0", closed: false}}})
    .mockRejectedValueOnce({code: "intentUnavailable", privateBody: "DO NOT SHOW"})
    .mockRejectedValueOnce({code: "creation", reason: {code: "storage", reason: {code: "DATA_SQL_UNAVAILABLE", reason: {
      operationId: identity.operationId, phase: "lookup", safeCause: "data_owner_unavailable", sqliteExtendedCode: 5, raw: "DO NOT SHOW"}}}})};
  await act(async () => root.render(<AgentChildCreationRecovery identity={identity} gateway={gateway} />));
  await click(); expect(await diagnostic()).toContain("CHILD_CREATION_IDENTITY_MISMATCH");
  await click(); expect(await diagnostic()).toContain("CHILD_CREATION_IDENTITY_MISMATCH");
  await click(); expect(await diagnostic()).toContain("intentUnavailable"); expect(host.textContent).not.toContain("DO NOT SHOW");
  await click(); const details = await diagnostic(); expect(details).toContain("DATA_SQL_UNAVAILABLE"); expect(details).toContain("data_owner_unavailable");
  expect(details).toContain("operation="); expect(details).not.toContain("DO NOT SHOW");
});

it("ignores a late reply after switching the original call", async () => {
  let finish!: (value: AgentChildCreationStatusResponse) => void;
  const gateway = {read: vi.fn(() => new Promise<AgentChildCreationStatusResponse>(resolve => {finish = resolve;}))};
  await act(async () => root.render(<AgentChildCreationRecovery identity={identity} gateway={gateway} />));
  await click();
  await act(async () => root.render(<AgentChildCreationRecovery identity={{...identity, callId: "77777777-7777-4777-8777-777777777777"}} gateway={gateway} />));
  await act(async () => finish(response()));
  expect(host.textContent).not.toContain("未找到原创建回执");
  expect(host.querySelector("button")?.disabled).toBe(false);
});
