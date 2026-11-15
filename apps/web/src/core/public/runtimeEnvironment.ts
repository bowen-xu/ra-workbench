import type { StructuredStartupImportResult } from "@research-assistant/core/structured-resources";
import { getAppRuntime, type AppRuntime } from "../runtime/appRuntime";
import type { NativeAgentPreview } from "../runtime/nativeAgentPreview";
import { ensureStructuredResourceStartup } from "../runtime/structuredResourceStartup";
export { hasPlatformCapabilities, isAppPreview } from "../runtime/appRuntime";
export { isDesktopRuntime } from "../runtime/platformCore";
export { freezeNewPreviewOwnerIntent, captureNewPreviewOwnerIntent, NATIVE_PREVIEW_OWNER_CAPABILITY } from "../runtime/nativePreviewOwnerIntent";

/** Runtime-selected adapter context, including the existing native preview
 * testing port. This grants no access to the application gateway registry,
 * runtime selection, application disposal or production credentials. */
export interface RuntimeEnvironment {
  readonly mode: "desktop" | "browser" | "preview";
  readonly storage: Storage;
  readonly nativeAgentPreview?: NativeAgentPreview;
  readonly hasStructuredResourceGateway: boolean;
}

const environments = new WeakMap<AppRuntime, RuntimeEnvironment>();
const owners = new WeakMap<RuntimeEnvironment, AppRuntime>();

export function getRuntimeEnvironment(): RuntimeEnvironment {
  const runtime = getAppRuntime();
  let environment = environments.get(runtime);
  if (!environment) {
    environment = {
      mode: runtime.mode,
      storage: runtime.storage,
      ...(runtime.nativeAgentPreview ? { nativeAgentPreview: runtime.nativeAgentPreview } : {}),
      hasStructuredResourceGateway: runtime.gateways.getStructuredResourceGateway !== undefined,
    };
    environments.set(runtime, environment);
    owners.set(environment, runtime);
  }
  return environment;
}

/** Keep the startup barrier attached to the captured runtime even if another
 * preview/storage is selected before this asynchronous operation begins. */
export function ensureRuntimeStructuredResources(environment: RuntimeEnvironment): Promise<StructuredStartupImportResult> {
  const runtime = owners.get(environment);
  if (!runtime) throw new Error("runtime_environment_unavailable: 运行环境上下文不可用。");
  return ensureStructuredResourceStartup(runtime);
}
