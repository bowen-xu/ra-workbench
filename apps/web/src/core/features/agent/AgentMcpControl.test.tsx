/** @vitest-environment happy-dom */
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { AgentMcpControl } from "./AgentMcpControl";
import type { AgentMcpServerSummary } from "../../services/agentGateway";

const gateway = vi.hoisted(() => ({ list: vi.fn(), install: vi.fn(), revoke: vi.fn() }));
const notifications = vi.hoisted(() => ({ error: vi.fn(), success: vi.fn() }));
vi.mock("../../services/agentGateway", () => ({ agentMcpGateway: gateway }));
vi.mock("../../services/notifications", () => ({ notify: notifications }));
(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const server: AgentMcpServerSummary = { serverId: "paper-search", environmentId: "local", revision: "r1", enabled: true, credentialConfigured: true, approved: false };
afterEach(() => { document.body.replaceChildren(); vi.clearAllMocks(); });

async function mount() {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  await act(async () => root.render(createElement(AgentMcpControl)));
  const trigger = document.querySelector<HTMLButtonElement>("button")!;
  await act(async () => { trigger.focus(); trigger.click(); });
  await act(async () => { await new Promise<void>((resolve) => requestAnimationFrame(() => resolve())); });
  return { host, root, trigger, item: () => document.querySelector<HTMLButtonElement>('[role="menuitem"]')! };
}

it("approves from a menu item without exposing server configuration or checkboxes", async () => {
  gateway.list.mockResolvedValue([server]); gateway.install.mockResolvedValue(undefined);
  const { root, trigger, item } = await mount();
  try {
    expect(document.body.textContent).toContain("paper-search");
    expect(document.body.textContent).not.toContain("local"); expect(document.body.textContent).not.toContain("r1");
    expect(document.querySelector("input")).toBeNull();
    await act(async () => item().click());
    expect(gateway.install).toHaveBeenCalledWith(["paper-search"]);
    expect(gateway.revoke).not.toHaveBeenCalled();
    expect(notifications.success).toHaveBeenCalled();
    expect(trigger.getAttribute("aria-expanded")).toBe("false");
  } finally { await act(async () => root.unmount()); }
});

it("filters disabled and uncredentialed servers from the approval set", async () => {
  gateway.list.mockResolvedValue([server,
    { ...server, serverId: "disabled-server", enabled: false, approved: true },
    { ...server, serverId: "missing-credential", credentialConfigured: false, approved: true },
  ]); gateway.install.mockResolvedValue(undefined);
  const { root, item } = await mount();
  try {
    expect([...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].slice(1).every((button) => button.disabled)).toBe(true);
    await act(async () => item().click());
    expect(gateway.install).toHaveBeenCalledWith(["paper-search"]);
  } finally { await act(async () => root.unmount()); }
});

it("revokes an approved tool directly", async () => {
  gateway.list.mockResolvedValue([{ ...server, approved: true }]); gateway.revoke.mockResolvedValue(undefined);
  const { root, item } = await mount();
  try {
    await act(async () => item().click());
    expect(gateway.revoke).toHaveBeenCalledWith("paper-search");
    expect(gateway.install).not.toHaveBeenCalled();
  } finally { await act(async () => root.unmount()); }
});

it("drops late list and update results after unmount", async () => {
  let resolveList!: (value: AgentMcpServerSummary[]) => void;
  gateway.list.mockImplementationOnce(() => new Promise((resolve) => { resolveList = resolve; }));
  const first = await mount();
  await act(async () => first.root.unmount());
  await act(async () => resolveList([server]));
  expect(document.body.textContent).not.toContain("paper-search");
  gateway.list.mockResolvedValue([server]);
  let resolveInstall!: () => void;
  gateway.install.mockImplementation(() => new Promise<void>((resolve) => { resolveInstall = resolve; }));
  const next = await mount();
  await act(async () => next.item().click());
  await act(async () => next.root.unmount());
  await act(async () => resolveInstall());
  expect(notifications.success).not.toHaveBeenCalled(); expect(notifications.error).not.toHaveBeenCalled();
});

it.each([true, false])("reconciles an in-flight change after reopening (success=%s)", async (success) => {
  let finish!: (success: boolean) => void;
  gateway.list.mockResolvedValue([server]);
  gateway.install.mockImplementation(() => new Promise<void>((resolve, reject) => { finish = (ok) => ok ? resolve() : reject(new Error("failed")); }));
  const { root, trigger, item } = await mount();
  try {
    await act(async () => item().click());
    await act(async () => { trigger.focus(); trigger.click(); });
    expect(item().disabled).toBe(true);
    gateway.list.mockResolvedValue([{ ...server, approved: success }]);
    await act(async () => finish(success));
    expect(item().disabled).toBe(false);
    expect(item().textContent).toContain(success ? "已批准" : "点击批准");
    expect(gateway.install).toHaveBeenCalledOnce();
  } finally { await act(async () => root.unmount()); }
});
