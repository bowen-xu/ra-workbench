import { isTauri } from "@tauri-apps/api/core";
import type { AppGateways } from "./appGatewayTypes";
import { getAppStorage, selectAppStorage } from "./appStorage";
import { invalidateConversationCatalog } from "./conversationCatalogEvents";
import type { BrowserAdapter } from "./browserAdapter";

export interface PreviewControls {
  readonly delayMs: number;
  readonly nextFailure: string | null;
  configure(values: { delayMs: number; nextFailure: string | null }): void;
  reset(): void | Promise<void>;
  dispose(): void;
  beforeOperation(name: string): Promise<void>;
}

export interface AppRuntime {
  readonly nativeAgentPreview?: import("./nativeAgentPreview").NativeAgentPreview;
  readonly events?: import("./platformEvents").PlatformEvents;
  readonly browser?: BrowserAdapter;
  readonly noteRecovery?: import("./noteRecoveryPreview").NoteRecoveryPreview;
  readonly notePreview?: import("./notePreviewScenarios").NotePreviewScenarios;
  readonly mode: "desktop" | "browser" | "preview";
  readonly storage: Storage;
  readonly gateways: Partial<AppGateways>;
  readonly preview?: PreviewControls;
  readonly invoke?: (command: string, args?: Record<string, unknown>, options?: import("./nativeAgentPreview").NativeAgentPreviewInvokeOptions) => Promise<unknown>;
  dispose(): void;
}

// Non-React services use the same startup-selected ports as the root provider.
// It is set before any business component mounts, never by an individual page.
let selected: AppRuntime | undefined;
export function selectAppRuntime(runtime: AppRuntime): void {
  selected = runtime;
  selectAppStorage(runtime.storage);
}
export function getAppRuntime(): AppRuntime {
  return selected ?? { mode: isTauri() ? "desktop" : "browser", get storage() { return getAppStorage(); }, gateways: {}, dispose() {} };
}
export function hasPlatformCapabilities(): boolean { return getAppRuntime().mode !== "browser"; }
export function isAppPreview(): boolean { return selected?.mode === "preview"; }
export function isNativeAgentPreview(): boolean { return selected?.nativeAgentPreview !== undefined; }

/** These production factories route every command through the selected HTTP
 * port. Other browser adapters must be supplied explicitly, never substituted. */
function nativeAgentFactory(name: keyof AppGateways): boolean {
  return name.startsWith("agent") || ["getAgentUndoEditorRestorationGateway", "getAiAssistanceGateway", "getWritingGateway", "getWritingCheckpointGateway", "textPatchGateway"].includes(name);
}

export function resolveAppGateway<K extends keyof AppGateways>(name: K, production: AppGateways[K]): AppGateways[K] {
  const override = selected?.gateways[name];
  if (selected?.mode === "preview" && override === undefined && (!selected.nativeAgentPreview || !nativeAgentFactory(name))) throw new Error(`预览未覆盖业务网关：${name}`);
  return override ?? production;
}

/** Stable exported gateway identities are used by app-owned queue controllers.
 * Resolve their implementation at invocation, rather than capturing native I/O
 * during module import. Missing preview Agent ports fail closed. */
export function runtimeAgentGateway<T extends object>(name: keyof AppGateways, production: T): T {
  const sessions = new WeakMap<object, { readonly runtime: AppRuntime | undefined; readonly proxy: object }>();
  return new Proxy(production, {
    get(target, property) {
      const gateway = selected?.gateways[name];
      if (selected?.mode === "preview" && !gateway && (!selected.nativeAgentPreview || !nativeAgentFactory(name)))
        return () => Promise.reject(new Error(`预览未覆盖 Agent 调用：${name}.${String(property)}`));
      const owner = gateway ?? target;
      const runtime = selected;
      const member = (object: object, key: string | symbol, prefix = ""): unknown => {
        const value: unknown = Reflect.get(object, key);
        const method = prefix + String(key);
        if (typeof value === "function") {
          if (!catalogMutations[name]?.includes(method)) return value.bind(object);
          return async (...args: unknown[]) => {
            try { return await value.apply(object, args); }
            finally { if (selected === runtime) invalidateConversationCatalog(); }
          };
        }
        // Preview context adapters expose their session port as a nested object.
        if (name === "agentContextGateway" && key === "sessions" && value && typeof value === "object") {
          const cached = sessions.get(value);
          if (cached && cached.runtime === runtime) return cached.proxy;
          const proxy = new Proxy(value, { get: (session, field) => member(session, field, "sessions.") });
          sessions.set(value, { runtime, proxy });
          return proxy;
        }
        return value;
      };
      return member(owner, property);
    },
  });
}

const catalogMutations: Partial<Record<keyof AppGateways, readonly string[]>> = {
  getAiAssistanceGateway: ["startPaperAnalysis", "startRun"],
  agentCatalogGateway: ["rename"], agentArchiveGateway: ["setArchived"], agentDeletionGateway: ["delete"], agentDeletionBatchGateway: ["deleteBatch"],
  agentCreationGateway: ["createBound"], agentSessionGateway: ["create"], agentForkGateway: ["fork"],
  agentChildSpawnGateway: ["spawnPaper", "spawnNote"], agentContextGateway: ["sessions.create"],
};
