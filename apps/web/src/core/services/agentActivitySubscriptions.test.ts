/** @vitest-environment happy-dom */
import { afterEach, expect, it, vi } from "vitest";
import { createAgentCatalogGateway, createAgentSubtaskGateway } from "@research-assistant/data/agent";
import { getAppRuntime, selectAppRuntime } from "../runtime/appRuntime";

const native = vi.hoisted(() => ({
  callbacks: new Map<string, (event: {payload: unknown}) => void>(),
  stop: vi.fn(),
  listen: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({listen: native.listen}));

import { agentCatalogGateway, agentSubtaskGateway } from "./agentGateway";

const original = getAppRuntime();
afterEach(() => { selectAppRuntime(original); native.callbacks.clear(); native.listen.mockReset(); native.stop.mockReset(); });

it("listens for native lifecycle changes and filters child state by parent", async () => {
  native.listen.mockImplementation((name: string, callback: (event: {payload: unknown}) => void) => {
    native.callbacks.set(name, callback); return Promise.resolve(native.stop);
  });
  selectAppRuntime({mode: "desktop", storage: localStorage, gateways: {}, dispose() {}});
  const activity = vi.fn(), children = vi.fn();
  const stopActivity = await agentCatalogGateway.subscribeActivity?.(activity);
  const stopChildren = await agentSubtaskGateway.subscribe?.("parent", children);
  native.callbacks.get("research-assistant:agent-activity-changed")?.({payload: {}});
  expect(activity).toHaveBeenCalledTimes(1);
  expect(activity).toHaveBeenLastCalledWith();
  native.callbacks.get("research-assistant:agent-subtasks-changed")?.({payload: {parentConversationId: "other"}});
  expect(children).not.toHaveBeenCalled();
  native.callbacks.get("research-assistant:agent-subtasks-changed")?.({payload: {parentConversationId: "parent"}});
  native.callbacks.get("research-assistant:agent-subtasks-changed")?.({payload: {}});
  expect(children).toHaveBeenCalledTimes(2);
  expect(children).toHaveBeenLastCalledWith();
  stopActivity?.(); stopChildren?.();
  expect(native.stop).toHaveBeenCalledTimes(2);
});

it("uses optional preview gateway listeners without falling back to native events", async () => {
  const stop = vi.fn(), subscribeActivity = vi.fn(async () => stop), subscribe = vi.fn(async () => stop);
  const catalog = createAgentCatalogGateway(vi.fn()), subtasks = createAgentSubtaskGateway(vi.fn());
  selectAppRuntime({mode: "preview", storage: localStorage, gateways: {
    agentCatalogGateway: {...catalog, subscribeActivity}, agentSubtaskGateway: {...subtasks, subscribe},
  }, dispose() {}});
  const changed = vi.fn();
  await agentCatalogGateway.subscribeActivity?.(changed);
  await agentSubtaskGateway.subscribe?.("parent", changed);
  expect(subscribeActivity).toHaveBeenCalledWith(changed);
  expect(subscribe).toHaveBeenCalledWith("parent", changed);
  selectAppRuntime({mode: "preview", storage: localStorage, gateways: {
    agentCatalogGateway: catalog, agentSubtaskGateway: subtasks,
  }, dispose() {}});
  expect(agentCatalogGateway.subscribeActivity).toBeUndefined();
  expect(agentSubtaskGateway.subscribe).toBeUndefined();
  expect(native.listen).not.toHaveBeenCalled();
});
