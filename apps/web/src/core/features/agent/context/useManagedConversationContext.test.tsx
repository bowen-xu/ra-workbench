// @vitest-environment happy-dom
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { EMPTY_AGENT_CONTEXT, type AgentContextConfigurationSaveIntent, type AgentContextState } from "@research-assistant/core/agent";
import type { AgentContextGateway } from "../../../services/agentContextGateway";
import { selectAppStorage } from "../../../runtime/appStorage";
import { createMockContextConfigurationAuthority } from "../../../../app/runtime/mock/mockContextConfigurationAuthority";
import { useNativeConversationContext } from "./useNativeConversationContext";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
const id = "11111111-1111-4111-8111-111111111111";
const edited: AgentContextState = { ...EMPTY_AGENT_CONTEXT, persistent: [{ label: "Synthetic", resource: { kind: "paperMetadata", resourceId: "synthetic" } }] };
let dispose: (() => Promise<void>) | undefined;
afterEach(async () => { await dispose?.(); dispose = undefined; localStorage.clear(); vi.unstubAllGlobals(); document.body.innerHTML = ""; });
function installLocks() {
  const held = new Set<string>();
  vi.stubGlobal("navigator", { locks: { request: async (name: string, _options: unknown, callback: (lock: object | null) => Promise<unknown>) => {
    if (held.has(name)) return callback(null); held.add(name); try { return await callback({}); } finally { held.delete(name); }
  } } });
}
async function fixture(failFirst: unknown = undefined, delayedSubscription = false, subscriptionInitiallyFails = false, commitBeforeFailure = false) {
  localStorage.clear(); selectAppStorage(localStorage); installLocks();
  const rows = new Map([[id, { revision: 1, state: EMPTY_AGENT_CONTEXT }]]);
  const authority = await createMockContextConfigurationAuthority(localStorage, rows);
  const calls: AgentContextConfigurationSaveIntent[] = [];
  let environmentFailure = false, subscriptionFailure = subscriptionInitiallyFails, subscriptionAttempts = 0;
  const environmentPauses: {reached: () => void; wait: Promise<void>}[] = [];
  let pause: Promise<void> | undefined, release: (() => void) | undefined;
  const configurationSave = { ...authority.configurationSave, environment: async (...args: Parameters<typeof authority.configurationSave.environment>) => {
    if (environmentFailure) throw new Error("environment unavailable");
    const result = await authority.configurationSave.environment(...args);
    const next = environmentPauses.shift(); if (next) { next.reached(); await next.wait; }
    return result;
  }, save: async (intent: AgentContextConfigurationSaveIntent) => {
    calls.push(structuredClone(intent)); if (pause) await pause;
    if (failFirst !== undefined) { const failure = failFirst; failFirst = undefined; if (commitBeforeFailure) await authority.configurationSave.save(intent); throw failure; }
    return authority.configurationSave.save(intent);
  } };
  const readConfiguration = vi.fn(async () => ({ revision: 1, state: EMPTY_AGENT_CONTEXT }));
  const saveConfiguration = vi.fn();
  let registerSubscription: (() => void) | undefined, changedConfiguration: (() => void) | undefined;
  const subscription = delayedSubscription ? new Promise<() => void>(resolve => { registerSubscription = () => resolve(() => undefined); }) : undefined;
  const gateway = { ...authority, configurationSave, readConfiguration, saveConfiguration,
    ...(subscription ? { subscribeConfiguration: (_id: string, callback: () => void) => {
      subscriptionAttempts++; changedConfiguration = callback;
      return subscriptionFailure ? Promise.reject(new Error("subscription unavailable")) : subscription;
    } } : {}) } as unknown as AgentContextGateway;
  const changed = vi.fn(), failed = vi.fn();
  let hook!: ReturnType<typeof useNativeConversationContext>;
  const frames: {readOnly: boolean; pending: boolean; ready: boolean; loading: boolean}[] = [];
  function Harness({ readOnly = false, pending = false }: { readonly readOnly?: boolean; readonly pending?: boolean }) {
    hook = useNativeConversationContext(id, pending, gateway, changed, failed, { readOnly });
    frames.push({ readOnly, pending, ready: hook.ready, loading: hook.loading }); return null;
  }
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  dispose = () => act(async () => root.unmount());
  await act(async () => { root.render(<StrictMode><Harness /></StrictMode>); });
  return { get hook() { return hook; }, calls, rows, readConfiguration, saveConfiguration, changed, failed, frames,
    hold: () => { pause = new Promise<void>(resolve => { release = resolve; }); },
    release: () => { const notify = release; pause = undefined; notify?.(); }, registerSubscription: () => registerSubscription?.(),
    failEnvironment: (fail: boolean) => { environmentFailure = fail; }, notifyConfiguration: () => changedConfiguration?.(),
    failSubscription: (fail: boolean) => { subscriptionFailure = fail; }, get subscriptionAttempts() { return subscriptionAttempts; },
    holdEnvironment: () => {
      let reached!: () => void, release!: () => void;
      const entered = new Promise<void>(resolve => { reached = resolve; });
      const wait = new Promise<void>(resolve => { release = resolve; });
      environmentPauses.push({reached, wait}); return {entered, release};
    },
    render: async (readOnly: boolean, pending = false) => act(async () => root.render(<StrictMode><Harness readOnly={readOnly} pending={pending} /></StrictMode>)) };
}

it("treats the first writable render after read-only as loading until context is restored", async () => {
  const test = await fixture();
  await test.render(true);
  const previousFrames = test.frames.length;
  await test.render(false);
  expect(test.frames.slice(previousFrames).filter(frame => !frame.readOnly))
    .not.toContainEqual({ readOnly: false, pending: false, ready: false, loading: false });
});

it("does not reuse pending readiness when the conversation becomes active", async () => {
  const test = await fixture();
  await test.render(false, true);
  const previousFrames = test.frames.length;
  await test.render(false, false);
  expect(test.frames[previousFrames]).toEqual({ readOnly: false, pending: false, ready: false, loading: true });
  expect(test.hook.ready).toBe(true);
});

it("uses HBS and the proven gateway for production context saves", async () => {
  const test = await fixture();
  expect(test.hook.ready).toBe(true); expect(test.readConfiguration).not.toHaveBeenCalled();
  await act(async () => test.hook.save(edited));
  expect(test.calls).toHaveLength(1); expect(test.saveConfiguration).not.toHaveBeenCalled();
  expect(test.rows.get(id)?.state).toEqual(edited); expect(test.hook.ready).toBe(true);
});

it("catches an authority update between initial hydration and subscription registration", async () => {
  const test = await fixture(undefined, true);
  expect(test.hook.ready).toBe(false); expect(test.hook.loading).toBe(true);
  test.rows.set(id, { revision: 2, state: edited });
  await act(async () => { test.registerSubscription(); await new Promise(resolve => setTimeout(resolve, 0)); });
  expect(test.changed).toHaveBeenLastCalledWith(edited);
  expect(test.hook.ready).toBe(true);
});

it("keeps the readiness barrier while notifications queued during catch-up are still being read", async () => {
  const test = await fixture(undefined, true);
  const first = test.holdEnvironment(), second = test.holdEnvironment();
  await act(async () => { test.registerSubscription(); await first.entered; });
  test.rows.set(id, {revision:2, state:edited});
  await act(async () => { test.notifyConfiguration(); first.release(); await second.entered; });
  expect(test.hook.ready).toBe(false);
  await act(async () => { second.release(); await new Promise(resolve => setTimeout(resolve, 0)); });
  expect(test.changed).toHaveBeenLastCalledWith(edited); expect(test.hook.ready).toBe(true);
});

it("does not unblock after a failed refresh until the authority has been read successfully", async () => {
  const test = await fixture(undefined, true);
  await act(async () => { test.registerSubscription(); await new Promise(resolve => setTimeout(resolve, 0)); });
  test.rows.set(id, { revision: 2, state: edited }); test.failEnvironment(true);
  await act(async () => { test.notifyConfiguration(); await new Promise(resolve => setTimeout(resolve, 0)); });
  expect(test.hook.recovery).toBe("error"); expect(test.hook.ready).toBe(false);
  await act(async () => { await expect(test.hook.retry()).rejects.toThrow("environment unavailable"); });
  expect(test.hook.recovery).toBe("error"); expect(test.hook.ready).toBe(false);
  test.failEnvironment(false);
  await act(async () => test.hook.retry());
  expect(test.changed).toHaveBeenLastCalledWith(edited); expect(test.hook.ready).toBe(true);
});

it("reconnects a failed subscription and catches up before returning to ready", async () => {
  const test = await fixture(undefined, true, true);
  expect(test.hook.recovery).toBe("error"); expect(test.hook.ready).toBe(false);
  await act(async () => { await expect(test.hook.retry()).rejects.toThrow("subscription unavailable"); });
  expect(test.hook.ready).toBe(false);
  test.rows.set(id, { revision: 2, state: edited });
  test.failSubscription(false); test.registerSubscription(); test.failEnvironment(true);
  await act(async () => { await expect(test.hook.retry()).rejects.toThrow("environment unavailable"); });
  expect(test.hook.ready).toBe(false);
  const connectedAttempts = test.subscriptionAttempts; test.failEnvironment(false);
  await act(async () => test.hook.retry());
  expect(connectedAttempts).toBeGreaterThanOrEqual(3); expect(test.subscriptionAttempts).toBe(connectedAttempts);
  expect(test.changed).toHaveBeenLastCalledWith(edited); expect(test.hook.ready).toBe(true);
  const later: AgentContextState = { ...EMPTY_AGENT_CONTEXT, persistent: [{ label: "Later", resource: { kind: "paperMetadata", resourceId: "later" } }] };
  test.rows.set(id, {revision:3, state:later});
  await act(async () => { test.notifyConfiguration(); await new Promise(resolve => setTimeout(resolve, 0)); });
  expect(test.changed).toHaveBeenLastCalledWith(later);
});

it("exposes a lost receipt and retries the exact original request", async () => {
  const test = await fixture(new Error("lost response"));
  await act(async () => { await expect(test.hook.save(edited)).rejects.toThrow("CONTEXT_CONFIGURATION_SAVE_UNCONFIRMED"); });
  expect(test.hook.recovery).toBe("save"); expect(test.hook.ready).toBe(false);
  await act(async () => test.hook.retry());
  expect(test.calls[1]).toEqual(test.calls[0]);
  expect(test.rows.get(id)?.state).toEqual(edited); expect(test.hook.ready).toBe(true);
});

it("catches an authority change notified while a committed save receipt is unknown", async () => {
  const test = await fixture(new Error("lost response"), true, false, true);
  await act(async () => { test.registerSubscription(); await new Promise(resolve => setTimeout(resolve, 0)); });
  await act(async () => { await expect(test.hook.save(edited)).rejects.toThrow(); });
  const remote: AgentContextState = {...EMPTY_AGENT_CONTEXT, persistent:[{label:"Remote",resource:{kind:"paperMetadata",resourceId:"remote"}}]};
  test.rows.set(id, {revision:3, state:remote});
  await act(async () => { test.notifyConfiguration(); await new Promise(resolve => setTimeout(resolve, 0)); });
  expect(test.hook.ready).toBe(false); expect(test.hook.recovery).toBe("save");
  await act(async () => test.hook.retry());
  expect(test.changed).toHaveBeenLastCalledWith(remote);
  expect(test.hook.ready).toBe(true);
});

it("retains safe native storage diagnostics for an unknown save", async () => {
  const failure = { code: "DATA_SQL_COMMIT_UNKNOWN", reason: { operationId: "44444444-4444-4444-8444-444444444444",
    phase: "commit", safeCause: "sql_commit_outcome_unknown", sqliteExtendedCode: 787 } };
  const test = await fixture(failure);
  await act(async () => { await expect(test.hook.save(edited)).rejects.toThrow(); });
  expect(test.hook.recovery).toBe("save");
  expect(test.hook.detail).toContain("DATA_SQL_COMMIT_UNKNOWN");
  expect(test.hook.detail).toContain("sqlite=787");
  expect(test.hook.detail).toContain(failure.reason.operationId);
});

it("exposes a concurrent context conflict and applies an explicit HBS choice", async () => {
  const test = await fixture(); test.hold();
  let saving!: Promise<void>;
  await act(async () => { saving = test.hook.save(edited); });
  for (let i = 0; i < 100 && !test.calls.length; i++) await Promise.resolve();
  expect(test.calls).toHaveLength(1);
  const remote: AgentContextState = { ...EMPTY_AGENT_CONTEXT, persistent: [{ label: "Remote", resource: { kind: "paperMetadata", resourceId: "remote" } }] };
  test.rows.set(id, { revision: 2, state: remote });
  await act(async () => { test.release(); await expect(saving).rejects.toThrow(); });
  expect(test.hook.recovery).toBe("conflict");
  const conflict = test.hook.conflicts[0]; expect(conflict).toBeDefined();
  await act(async () => test.hook.resolve(conflict!.id, "keep"));
  expect(test.rows.get(id)?.state).toEqual(edited);
  expect(test.hook.ready).toBe(true);
});

it("uses read-only hydration and refuses writes after archive", async () => {
  const test = await fixture();
  await test.render(true);
  expect(test.readConfiguration).toHaveBeenLastCalledWith(id, false);
  await expect(test.hook.save(edited)).rejects.toThrow("AGENT_CONTEXT_READ_ONLY");
  expect(test.calls).toHaveLength(0);
});
