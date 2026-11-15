/** @vitest-environment happy-dom */
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { ConversationMenu } from "./ConversationMenu";
import { notify } from "../../services/notifications";
import type { AgentConversationDownload } from "@research-assistant/core/agent";
vi.mock("../../services/agentGateway", () => ({ agentConversationExportGateway: {}, agentCatalogGateway: {} }));
vi.mock("../../services/notifications", () => ({ notify: { error: vi.fn(), success: vi.fn() } }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

it("completes menu pointer selection once and reports the actual Downloads path", async () => {
  vi.clearAllMocks();
  let resolve!: (value: AgentConversationDownload) => void;
  const download = vi.fn(() => new Promise<AgentConversationDownload>((done) => { resolve = done; }));
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<StrictMode><ConversationMenu conversationId="owner" title="Title" gateway={{ download, snapshot: vi.fn() }} /></StrictMode>));
  await act(async () => container.querySelector<HTMLButtonElement>("button")?.click());
  const item = document.querySelector<HTMLButtonElement>('[role="menuitem"]');
  expect(item).not.toBeNull();
  await act(async () => {
    item?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    item?.dispatchEvent(new FocusEvent("blur", { relatedTarget: null }));
    item?.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
    item?.click(); item?.click();
  });
  expect(download).toHaveBeenCalledExactlyOnceWith("owner");
  expect(container.querySelector<HTMLButtonElement>("button")?.disabled).toBe(true);
  await act(async () => resolve({ schemaVersion: 1, conversationId: "owner", watermark: 0, byteSize: 100, destinationPath: "/Downloads/Title (1).json" }));
  expect(notify.success).toHaveBeenCalledWith("“Title”对话已下载到 Downloads", { description: "/Downloads/Title (1).json" });
  await act(async () => root.unmount()); container.remove();
});

it("reports a failed download without success or automatic retry", async () => {
  vi.clearAllMocks();
  const download = vi.fn().mockRejectedValue("tooLarge");
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<ConversationMenu conversationId="owner" title="Title" gateway={{ download, snapshot: vi.fn() }} />));
  await act(async () => container.querySelector<HTMLButtonElement>("button")?.click());
  await act(async () => document.querySelector<HTMLButtonElement>('[role="menuitem"]')?.click());
  expect(download).toHaveBeenCalledTimes(1);
  expect(notify.success).not.toHaveBeenCalled();
  expect(notify.error).toHaveBeenCalledWith("对话历史超过导出上限，未生成下载文件。");
  await act(async () => root.unmount()); container.remove();
});

it("drops a late download result after the menu is unmounted", async () => {
  vi.clearAllMocks();
  let resolve!: (value: AgentConversationDownload) => void;
  const download = vi.fn(() => new Promise<AgentConversationDownload>((done) => { resolve = done; }));
  const container = document.createElement("div"); document.body.append(container);
  const root = createRoot(container);
  await act(async () => root.render(<ConversationMenu conversationId="owner" title="Title" gateway={{ download, snapshot: vi.fn() }} />));
  await act(async () => container.querySelector<HTMLButtonElement>("button")?.click());
  await act(async () => document.querySelector<HTMLButtonElement>('[role="menuitem"]')?.click());
  await act(async () => root.unmount());
  await act(async () => resolve({ schemaVersion: 1, conversationId: "owner", watermark: 0, byteSize: 100, destinationPath: "/Downloads/Title.json" }));
  expect(notify.success).not.toHaveBeenCalled(); expect(notify.error).not.toHaveBeenCalled();
  container.remove();
});
