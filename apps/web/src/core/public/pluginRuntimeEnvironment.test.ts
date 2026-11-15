import {beforeEach, describe, expect, it, vi} from "vitest";
import type {AppRuntime} from "../runtime/appRuntime";
import {
  ensureRuntimeStructuredResources,
  getRuntimeEnvironment,
  hasPlatformCapabilities,
  isAppPreview,
  isDesktopRuntime,
  getNativePreviewCapabilities,
  getRuntimeStorage,
  captureNewPreviewOwnerIntent,
  freezeNewPreviewOwnerIntent,
} from "./pluginRuntimeEnvironment";
import {NATIVE_PREVIEW_OWNER_CAPABILITY, readPreviewOwnerIntent} from "../runtime/nativePreviewOwnerIntent";
import {registerStoragePersistence} from "../runtime/storagePersistence";

const state = vi.hoisted(() => ({
  runtime: undefined as AppRuntime | undefined,
  startup: vi.fn(async (_runtime: AppRuntime) => ({paperIds: [], importedPaperCount: 0, importedTemplateCount: 0})),
}));
vi.mock("../runtime/appRuntime", () => ({
  getAppRuntime: () => state.runtime,
  isAppPreview: () => state.runtime?.mode === "preview",
  hasPlatformCapabilities: () => state.runtime?.mode !== "browser",
}));
vi.mock("../runtime/structuredResourceStartup", () => ({ensureStructuredResourceStartup: state.startup}));

function runtime(mode: AppRuntime["mode"] = "preview"): AppRuntime {
  return {mode, storage: {} as Storage, gateways: {}, dispose: vi.fn(), nativeAgentPreview: {
    bootstrap: {schemaVersion: 1, generation: "synthetic", model: {providerId: "synthetic", modelId: "synthetic", revision: null},
      manuscriptId: "synthetic", mechanism: "native-agent-raft-sqlite", supported: [], pending: []},
    invoke: vi.fn(async () => undefined), events: {listen: vi.fn(async () => () => undefined)},
    configure: vi.fn(async () => undefined), reconnect: vi.fn(), reset: vi.fn(async () => undefined),
    subscribe: vi.fn(() => () => undefined), getSnapshot: () => ({error: null}), dispose: vi.fn(),
  }};
}

beforeEach(() => {state.startup.mockClear();});

describe("narrow plugin runtime environment", () => {
  it("publishes only two frozen own fields without storage, native ports or hidden symbols", () => {
    const unused = async (): Promise<never> => {throw new Error("not called");};
    state.runtime = {...runtime(), gateways: {getStructuredResourceGateway: {
      createTemplate: unused, list: unused, get: unused, getSources: unused, getVersion: unused,
      write: unused, review: unused, restoreHistory: unused, bindTemplate: unused,
      importLegacy: unused, importStartup: unused,
    }}};
    const view = getRuntimeEnvironment();
    expect(Reflect.ownKeys(view)).toEqual(["mode", "hasStructuredResourceGateway"]);
    expect(view).toEqual({mode: "preview", hasStructuredResourceGateway: true});
    expect(Object.isFrozen(view)).toBe(true);
    expect(getRuntimeEnvironment()).toBe(view);
    expect(view).not.toHaveProperty("storage");
    expect(view).not.toHaveProperty("nativeAgentPreview");
    expect(view).not.toHaveProperty("invoke");
    expect(state.startup).not.toHaveBeenCalled();
  });

  it("rejects forged and cloned views before invoking the original startup owner", () => {
    state.runtime = runtime();
    const view = getRuntimeEnvironment();
    for (const forged of [{...view}, Object.freeze({mode: "preview" as const, hasStructuredResourceGateway: false})]) {
      expect(() => ensureRuntimeStructuredResources(forged))
        .toThrowError("runtime_environment_unavailable: 运行环境上下文不可用。");
    }
    expect(state.startup).not.toHaveBeenCalled();
  });

  it("delegates to the originally captured owner after runtime selection changes", async () => {
    const first = runtime(), second = runtime("browser");
    state.runtime = first;
    const captured = getRuntimeEnvironment();
    state.runtime = second;
    const current = getRuntimeEnvironment();
    expect(current).not.toBe(captured);
    expect(current.mode).toBe("browser");
    await ensureRuntimeStructuredResources(captured);
    expect(state.startup).toHaveBeenCalledExactlyOnceWith(first);
  });

  it("forwards the existing boolean queries without constructing an environment owner", () => {
    state.runtime = runtime();
    expect([isAppPreview(), isDesktopRuntime(), hasPlatformCapabilities()]).toEqual([true, false, true]);
    state.runtime = runtime("desktop");
    expect([isAppPreview(), isDesktopRuntime(), hasPlatformCapabilities()]).toEqual([false, true, true]);
    state.runtime = runtime("browser");
    expect([isAppPreview(), isDesktopRuntime(), hasPlatformCapabilities()]).toEqual([false, false, false]);
    expect(state.startup).not.toHaveBeenCalled();
  });
});

function memoryStorage(): Storage {
  const values = new Map<string, string>();
  return {
    get length() {return values.size;},
    getItem: key => values.get(key) ?? null,
    setItem: (key, value) => {values.set(key, value);},
    removeItem: key => {values.delete(key);},
    key: index => [...values.keys()][index] ?? null,
    clear: () => {values.clear();},
  };
}
function persist(storage: Storage, flush = vi.fn(async () => {})) {
  const port = {backend: "IndexedDB" as const, pending: false, error: undefined as Error | undefined,
    resetting: false, flush, retry: vi.fn(async () => {}), reset: async () => {}, subscribe: () => () => {}, close() {}};
  registerStoragePersistence(storage, port);
  return port;
}
const conversation = "11111111-1111-4111-8111-111111111111";
const request = "22222222-2222-4222-8222-222222222222";
const fixture = "33333333-3333-4333-8333-333333333333";

describe("captured native preview capabilities and owner intent", () => {
  it("keeps preview presence distinct from support and reads one current bootstrap per query", () => {
    const first = runtime();
    let bootstrap = {...first.nativeAgentPreview!.bootstrap, supported: [] as string[]};
    const readBootstrap = vi.fn(() => bootstrap);
    Object.defineProperty(first.nativeAgentPreview, "bootstrap", {get: readBootstrap});
    state.runtime = first;
    const view = getRuntimeEnvironment();
    const initial = getNativePreviewCapabilities(view);
    expect(initial).toEqual({generation: "synthetic", supportsManualCompaction: false, supportsOwnerIntent: false});
    expect(Object.isFrozen(initial)).toBe(true);
    expect(readBootstrap).toHaveBeenCalledTimes(1);
    expect(Reflect.ownKeys(view)).toEqual(["mode", "hasStructuredResourceGateway"]);
    bootstrap = {...bootstrap, generation: "reset", supported: ["manual-compaction", NATIVE_PREVIEW_OWNER_CAPABILITY]};
    state.runtime = runtime("desktop");
    expect(getNativePreviewCapabilities(view)).toEqual({generation: "reset", supportsManualCompaction: true, supportsOwnerIntent: true});
    expect(readBootstrap).toHaveBeenCalledTimes(2);
    expect(initial?.generation).toBe("synthetic");
    const absent: AppRuntime = {mode: "preview", storage: {} as Storage, gateways: {}, dispose() {}};
    state.runtime = absent;
    expect(getNativePreviewCapabilities(getRuntimeEnvironment())).toBeUndefined();
  });

  it("rejects cloned views before querying or capturing any native owner", async () => {
    state.runtime = runtime();
    const snapshot = vi.fn();
    state.runtime.nativeAgentPreview!.snapshotConversationOwner = snapshot;
    const view = getRuntimeEnvironment(), cloned = {...view};
    expect(() => getRuntimeStorage(cloned)).toThrow("runtime_environment_unavailable");
    expect(() => getNativePreviewCapabilities(cloned)).toThrow("runtime_environment_unavailable");
    expect(() => captureNewPreviewOwnerIntent(cloned, conversation)).toThrow("runtime_environment_unavailable");
    await expect(freezeNewPreviewOwnerIntent(cloned, conversation, request)).rejects.toThrow("runtime_environment_unavailable");
    expect(snapshot).not.toHaveBeenCalled();
  });

  it("leaves unsupported owners untouched before checking IDs, snapshot support or storage", async () => {
    state.runtime = runtime();
    const view = getRuntimeEnvironment();
    expect(captureNewPreviewOwnerIntent(view, "invalid")).toBeUndefined();
    await expect(freezeNewPreviewOwnerIntent(view, "invalid", "invalid")).resolves.toBeUndefined();
    expect(state.runtime.storage).toEqual({});
  });

  it("preserves snapshot, persistence and UUID exception priority", async () => {
    const selected = {...runtime(), storage: memoryStorage()};
    Object.assign(selected.nativeAgentPreview!.bootstrap, {supported: [NATIVE_PREVIEW_OWNER_CAPABILITY]});
    state.runtime = selected;
    const view = getRuntimeEnvironment();
    expect(() => captureNewPreviewOwnerIntent(view, "invalid")).toThrow("agent_preview_owner_unavailable");
    const failure = new Error("synthetic_snapshot_failure");
    const snapshot = vi.fn<(conversationId: string) => {generation: string; fixtureId: string | null}>(() => {throw failure;});
    selected.nativeAgentPreview!.snapshotConversationOwner = snapshot;
    expect(() => captureNewPreviewOwnerIntent(view, "invalid")).toThrow("agent_preview_owner_storage_unavailable");
    expect(snapshot).not.toHaveBeenCalled();
    persist(selected.storage);
    expect(() => captureNewPreviewOwnerIntent(view, "invalid")).toThrow(failure);
    snapshot.mockImplementation(() => ({generation: "synthetic", fixtureId: fixture}));
    expect(() => captureNewPreviewOwnerIntent(view, "invalid")).toThrow("agent_preview_owner_invalid");
    expect(snapshot).toHaveBeenLastCalledWith("invalid");
    const writer = captureNewPreviewOwnerIntent(view, conversation)!;
    await expect(writer("invalid")).rejects.toThrow("agent_preview_owner_invalid");
    expect(selected.storage.length).toBe(0);
  });

  it("retains the original owner and writer through storage selection, failure, retry and rebase", async () => {
    const selected = {...runtime(), storage: memoryStorage()};
    Object.assign(selected.nativeAgentPreview!.bootstrap, {supported: [NATIVE_PREVIEW_OWNER_CAPABILITY]});
    let active: string | null = fixture;
    const snapshot = vi.fn(() => ({generation: "synthetic", fixtureId: active}));
    selected.nativeAgentPreview!.snapshotConversationOwner = snapshot;
    const failure = new Error("synthetic_flush_failure");
    const flush = vi.fn(async () => {});
    const port = persist(selected.storage, flush);
    flush.mockImplementationOnce(async () => {port.error = failure; throw failure;});
    port.retry.mockImplementation(async () => {port.error = undefined;});
    state.runtime = selected;
    const view = getRuntimeEnvironment();
    const writer = captureNewPreviewOwnerIntent(view, conversation)!;
    active = null;
    const replacement = {...runtime(), storage: memoryStorage()};
    state.runtime = replacement;
    await expect(writer(request)).rejects.toBe(failure);
    await writer(request);
    const rebased = "44444444-4444-4444-8444-444444444444";
    await writer(rebased);
    expect(port.retry).toHaveBeenCalledOnce();
    expect(snapshot).toHaveBeenCalledOnce();
    expect(readPreviewOwnerIntent(selected.storage, conversation, request)).toEqual({fixtureId: fixture});
    expect(readPreviewOwnerIntent(selected.storage, conversation, rebased)).toEqual({fixtureId: fixture});
    expect(replacement.storage.length).toBe(0);
    await expect(freezeNewPreviewOwnerIntent(view, conversation, request)).rejects.toThrow("agent_preview_owner_intent_conflict");
  });

  it("preserves a real generation change racing with the original flush", async () => {
    const selected = {...runtime(), storage: memoryStorage()};
    let bootstrap = {...selected.nativeAgentPreview!.bootstrap, supported: [NATIVE_PREVIEW_OWNER_CAPABILITY]};
    Object.defineProperty(selected.nativeAgentPreview, "bootstrap", {get: () => bootstrap});
    selected.nativeAgentPreview!.snapshotConversationOwner = () => ({generation: "synthetic", fixtureId: fixture});
    persist(selected.storage, vi.fn(async () => {bootstrap = {...bootstrap, generation: "after-reset"};}));
    state.runtime = selected;
    const view = getRuntimeEnvironment();
    await expect(freezeNewPreviewOwnerIntent(view, conversation, request)).rejects.toThrow("agent_preview_stale_owner");
    expect(readPreviewOwnerIntent(selected.storage, conversation, request)).toEqual({fixtureId: fixture});
    expect(getNativePreviewCapabilities(view)?.generation).toBe("after-reset");
  });
});
