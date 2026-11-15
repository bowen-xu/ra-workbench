/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ConversationDeleteDemo } from "./ConversationDeleteDemo";
import { createConversationDeletePreviewData } from "./conversationDeletePreviewData";
import { notify } from "../../../core/services/notifications";
import { mockWebLocks } from "../../../core/runtime/testing/mockWebLocks";

vi.mock("./conversationDeletePreviewData", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./conversationDeletePreviewData")>();
  return { ...actual, createConversationDeletePreviewData: vi.fn((scenario) => actual.createConversationDeletePreviewData(scenario, 0)) };
});
vi.mock("../../../core/services/notifications", () => ({ replayVisibleNotifications: vi.fn(), notify: { success: vi.fn(), error: vi.fn() } }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root;
const preferences = new Map<string, string>();
beforeEach(() => {
  mockWebLocks();
  Object.defineProperty(window, "localStorage", { configurable: true, value: {
    getItem: (key: string) => preferences.get(key) ?? null,
    setItem: (key: string, value: string) => { preferences.set(key, value); },
    removeItem: (key: string) => { preferences.delete(key); },
  } });
  vi.spyOn(HTMLDialogElement.prototype, "showModal").mockImplementation(function (this: HTMLDialogElement) { this.setAttribute("open", ""); });
});
afterEach(async () => {
  await act(async () => root?.unmount());
  document.body.replaceChildren(); preferences.clear(); vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.clearAllMocks();
});
function button(label: string): HTMLButtonElement {
  const result = [...document.querySelectorAll<HTMLButtonElement>("button")].find((item) => item.textContent === label);
  if (!result) throw new Error(`Missing button: ${label}`);
  return result;
}
async function retryUnknown(): Promise<void> {
  await act(async () => button("核对原删除").click());
  await act(async () => button("重试原删除").click());
}
async function mount() {
  const host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await act(async () => root.render(<ConversationDeleteDemo />));
}
async function openDeletion() {
  const trigger = document.querySelector<HTMLButtonElement>('[aria-label="永久删除对话"]');
  if (!trigger) throw new Error("Missing production deletion entry");
  await act(async () => trigger.click());
}
async function scenario(value: string) {
  const select = document.querySelector<HTMLSelectElement>("select")!;
  await act(async () => { select.value = value; select.dispatchEvent(new Event("change", { bubbles: true })); });
}
function data() { return vi.mocked(createConversationDeletePreviewData).mock.results.at(-1)!.value as ReturnType<typeof createConversationDeletePreviewData>; }

it("opens the production dialog from the archived list, focuses cancel, deletes mock rows and resets", async () => {
  await mount();
  expect(document.querySelector('[aria-label="返回当前对话"]')).not.toBeNull();
  expect(document.querySelectorAll(".conversation-list-row")).toHaveLength(3);
  await openDeletion();
  expect(document.activeElement).toBe(button("取消"));
  expect(data().attempts).toHaveLength(0);
  await act(async () => button("取消").click());
  expect(document.querySelector("dialog")).toBeNull();
  await openDeletion(); await act(async () => button("确认").click());
  expect(document.querySelector("dialog")).toBeNull();
  expect(document.querySelectorAll(".conversation-list-row")).toHaveLength(2);
  await act(async () => button("重置 mock 数据").click());
  expect(document.querySelectorAll(".conversation-list-row")).toHaveLength(3);
});

it.each(["branchHistory", "proposalApplications", "executionWorkspace"])("exercises production retention and separate force confirmation: %s", async (value) => {
  await mount(); await scenario(value); await openDeletion();
  await act(async () => button("确认").click());
  expect(document.querySelector('dialog[aria-label="永久删除对话"]')).toBeNull();
  expect(document.querySelector('dialog[aria-label="强制删除对话"]')).not.toBeNull();
  expect(document.querySelectorAll("dialog")).toHaveLength(1);
  expect(document.activeElement).toBe(button("取消"));
  expect(document.querySelectorAll(".conversation-list-row")).toHaveLength(3);
  expect(data().attempts).toHaveLength(1);
  await act(async () => button("确认").click());
  expect(data().attempts.map((request) => request.force)).toEqual([false, true]);
  expect(document.querySelectorAll(".conversation-list-row")).toHaveLength(2);
});

it("retries the same frozen request after an uncertain mock result", async () => {
  await mount(); await scenario("unavailable"); await openDeletion();
  await act(async () => {
    document.querySelector<HTMLInputElement>('.conversation-delete-skip input')!.click();
    button("确认").click();
  });
  expect(document.querySelector("dialog")).not.toBeNull();
  expect(document.querySelector('[role="alert"]')?.textContent).toContain("删除失败或结果不明，请先核对原请求。");
  expect(document.querySelector<HTMLInputElement>('.conversation-delete-skip input')!.checked).toBe(true);
  expect(data().deletionConfirmationPreferences.shouldSkip()).toBe(true);
  await retryUnknown();
  expect(data().attempts[0]).toEqual(data().attempts[1]);
  expect(document.querySelector("dialog")).toBeNull();
  expect(data().deletionConfirmationPreferences.shouldSkip()).toBe(true);
});

it.each(["busy", "conflict"])("keeps the mock conversation after %s and preserves the production retry rule", async (value) => {
  await mount(); await scenario(value); await openDeletion();
  await act(async () => button("确认").click());
  expect(document.querySelectorAll(".conversation-list-row")).toHaveLength(3);
  expect(button("重试").disabled).toBe(value === "conflict");
  expect(button("取消").disabled).toBe(false);
});

it("keeps suppression after a failed retry and cancel, and only restores confirmation when explicitly unchecked", async () => {
  await mount(); await scenario("busy"); await openDeletion();
  await act(async () => document.querySelector<HTMLInputElement>('.conversation-delete-skip input')!.click());
  expect(data().deletionConfirmationPreferences.shouldSkip()).toBe(true);
  expect(data().attempts).toHaveLength(0);
  await act(async () => button("确认").click());
  await act(async () => button("重试").click());
  expect(data().attempts[0]).toEqual(data().attempts[1]);
  await act(async () => button("取消").click());
  expect(data().deletionConfirmationPreferences.shouldSkip()).toBe(true);
  await openDeletion();
  expect(data().attempts).toHaveLength(3);
  expect(button("重试").disabled).toBe(false);
  expect(document.querySelector<HTMLInputElement>('.conversation-delete-skip input')!.checked).toBe(true);
  await act(async () => document.querySelector<HTMLInputElement>('.conversation-delete-skip input')!.click());
  expect(data().deletionConfirmationPreferences.shouldSkip()).toBe(false);
  await act(async () => button("取消").click());
  await openDeletion();
  expect(data().attempts).toHaveLength(3);
  expect(button("确认").disabled).toBe(false);
  expect(document.querySelector<HTMLInputElement>('.conversation-delete-skip input')!.checked).toBe(false);
});

it("isolates today's skip preference from application storage and resets it with the mock data", async () => {
  const key = "research-assistant.agent.delete-confirmation-skip-date";
  const now = new Date();
  preferences.set(key, `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`);
  const original = preferences.get(key);
  await mount(); await openDeletion();
  expect(data().attempts).toHaveLength(0);
  await act(async () => {
    document.querySelector<HTMLInputElement>('.conversation-delete-skip input')!.click();
    button("确认").click();
  });
  expect(data().deletionConfirmationPreferences.shouldSkip()).toBe(true);
  expect(preferences.get(key)).toBe(original);
  await openDeletion();
  expect(document.querySelector("dialog")).toBeNull();
  expect(document.querySelectorAll(".conversation-list-row")).toHaveLength(1);
  await act(async () => button("重置 mock 数据").click());
  await openDeletion();
  expect(document.querySelector("dialog")).not.toBeNull();
  expect(preferences.get(key)).toBe(original);
});

it("stores ordinary and force checkbox preferences independently of the deletion result", async () => {
  await mount(); await scenario("branchHistory"); await openDeletion();
  await act(async () => {
    document.querySelector<HTMLInputElement>('.conversation-delete-skip input')!.click();
    button("确认").click();
  });
  expect(data().deletionConfirmationPreferences.shouldSkip()).toBe(true);
  expect(document.querySelector<HTMLInputElement>('.conversation-delete-skip input')!.checked).toBe(false);
  await act(async () => {
    document.querySelector<HTMLInputElement>('.conversation-delete-skip input')!.click();
    button("确认").click();
  });
  expect(data().forceDeletionConfirmationPreferences.shouldSkip()).toBe(true);
  expect(data().deletionConfirmationPreferences.shouldSkip()).toBe(true);
  await openDeletion();
  expect(document.querySelector("dialog")).toBeNull();
  expect(data().attempts.map((request) => request.force)).toEqual([false, true, false, true]);
  const successes = vi.mocked(notify.success).mock.calls;
  expect(notify.error).not.toHaveBeenCalled();
  expect(successes).toHaveLength(2);
  expect(successes.every((call) => call.length === 1)).toBe(true);
  expect(successes[0]?.[0]).not.toBe(successes[1]?.[0]);
  expect(document.querySelectorAll(".conversation-list-row")).toHaveLength(1);
  await act(async () => button("重置 mock 数据").click());
  expect(data().forceDeletionConfirmationPreferences.shouldSkip()).toBe(false);
  await openDeletion(); await act(async () => button("确认").click());
  expect(document.querySelector('dialog[aria-label="强制删除对话"]')).not.toBeNull();
});

it("remembers the ordinary choice after force deletion without suppressing force confirmation", async () => {
  await mount(); await scenario("executionWorkspace"); await openDeletion();
  await act(async () => {
    document.querySelector<HTMLInputElement>('.conversation-delete-skip input')!.click();
    button("确认").click();
  });
  expect(data().deletionConfirmationPreferences.shouldSkip()).toBe(true);
  expect(document.querySelector<HTMLInputElement>('.conversation-delete-skip input')!.checked).toBe(false);
  await act(async () => button("确认").click());
  expect(data().deletionConfirmationPreferences.shouldSkip()).toBe(true);
  expect(data().forceDeletionConfirmationPreferences.shouldSkip()).toBe(false);
  await openDeletion();
  expect(document.querySelector('dialog[aria-label="永久删除对话"]')).toBeNull();
  expect(document.querySelector('dialog[aria-label="强制删除对话"]')).not.toBeNull();
  expect(data().attempts.map((request) => request.force)).toEqual([false, true, false]);
});

it("keeps both preferences after cancelling force confirmation and skips both on the next deletion", async () => {
  await mount(); await scenario("executionWorkspace"); await openDeletion();
  await act(async () => {
    document.querySelector<HTMLInputElement>('.conversation-delete-skip input')!.click();
    button("确认").click();
  });
  await act(async () => {
    document.querySelector<HTMLInputElement>('.conversation-delete-skip input')!.click();
    button("取消").click();
  });
  expect(data().forceDeletionConfirmationPreferences.shouldSkip()).toBe(true);
  expect(data().deletionConfirmationPreferences.shouldSkip()).toBe(true);
  expect(document.querySelectorAll(".conversation-list-row")).toHaveLength(3);
  expect(data().attempts).toHaveLength(1);
  await openDeletion();
  expect(data().attempts.map((request) => request.force)).toEqual([false, false, true]);
  expect(document.querySelector('dialog[open]')).toBeNull();
  expect(document.querySelectorAll(".conversation-list-row")).toHaveLength(2);
});

it("does not skip the independent force modal when ordinary confirmation is suppressed", async () => {
  await mount(); await scenario("branchHistory");
  data().deletionConfirmationPreferences.setSkipToday(true);
  await openDeletion();
  expect(data().attempts).toHaveLength(1);
  expect(data().attempts[0]?.force).toBe(false);
  expect(document.querySelector('dialog[aria-label="强制删除对话"]')).not.toBeNull();
  expect(data().forceDeletionConfirmationPreferences.shouldSkip()).toBe(false);
});

it.each(["sqlUnknown","workspacePartial","workspaceUnknown"])("shows structured diagnostics through the production dialog and preserves the original mock request: %s",async value=>{
 await mount();await scenario(value);await openDeletion();await act(async()=>button("确认").click());
 if(value!=="sqlUnknown")await act(async()=>button("确认").click());
 const request=data().attempts.at(-1)!;const message=vi.mocked(notify.error).mock.calls.at(-1)![0];
 expect(message).toContain(request.requestId);expect(message).toContain(value==="sqlUnknown"?"DATA_SQL_COMMIT_UNKNOWN":value==="workspacePartial"?"DATA_OPERATION_PARTIAL":"DATA_OPERATION_OUTCOME_UNKNOWN");
 expect(document.querySelectorAll(".conversation-list-row")).toHaveLength(3);
 await retryUnknown();expect(data().attempts.at(-1)).toEqual(request);expect(document.querySelectorAll(".conversation-list-row")).toHaveLength(2);
});
