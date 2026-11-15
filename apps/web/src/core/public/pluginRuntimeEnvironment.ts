import type {StructuredStartupImportResult} from "@research-assistant/core/structured-resources";
import type {NativePreviewCapabilities, RuntimeEnvironment} from "@research-assistant/workbench-sdk/environment";
import {
  captureNewPreviewOwnerIntent as captureCorePreviewOwnerIntent,
  freezeNewPreviewOwnerIntent as freezeCorePreviewOwnerIntent,
  NATIVE_PREVIEW_OWNER_CAPABILITY,
} from "../runtime/nativePreviewOwnerIntent";
import {
  ensureRuntimeStructuredResources as ensureCapturedEnvironment,
  getRuntimeEnvironment as getCoreEnvironment,
  type RuntimeEnvironment as CoreEnvironment,
} from "./runtimeEnvironment";

export {hasPlatformCapabilities, isAppPreview, isDesktopRuntime} from "./runtimeEnvironment";
export type {NativePreviewCapabilities, RuntimeEnvironment} from "@research-assistant/workbench-sdk/environment";

const views = new WeakMap<CoreEnvironment, RuntimeEnvironment>();
const owners = new WeakMap<RuntimeEnvironment, CoreEnvironment>();

function capturedEnvironment(environment: RuntimeEnvironment): CoreEnvironment {
  const owner = owners.get(environment);
  if (!owner) throw new Error("runtime_environment_unavailable: 运行环境上下文不可用。");
  return owner;
}

/** Preserve Core's runtime identity without publishing its storage or preview
 * controls. Repeated reads of the same captured environment share this view. */
export function getRuntimeEnvironment(): RuntimeEnvironment {
  const environment = getCoreEnvironment();
  let view = views.get(environment);
  if (!view) {
    view = Object.freeze({
      mode: environment.mode,
      hasStructuredResourceGateway: environment.hasStructuredResourceGateway,
    });
    views.set(environment, view);
    owners.set(view, environment);
  }
  return view;
}

export function getRuntimeStorage(environment: RuntimeEnvironment): Storage {
  return capturedEnvironment(environment).storage;
}

export function getNativePreviewCapabilities(environment: RuntimeEnvironment): Readonly<NativePreviewCapabilities> | undefined {
  const preview = capturedEnvironment(environment).nativeAgentPreview;
  if (!preview) return undefined;
  const bootstrap = preview.bootstrap;
  return Object.freeze({
    generation: bootstrap.generation,
    supportsManualCompaction: bootstrap.supported.includes("manual-compaction"),
    supportsOwnerIntent: bootstrap.supported.includes(NATIVE_PREVIEW_OWNER_CAPABILITY),
  });
}

export function captureNewPreviewOwnerIntent(environment: RuntimeEnvironment,
  conversationId: string): ((requestId: string) => Promise<void>) | undefined {
  const owner = capturedEnvironment(environment);
  return captureCorePreviewOwnerIntent(owner.nativeAgentPreview, owner.storage, conversationId);
}

export async function freezeNewPreviewOwnerIntent(environment: RuntimeEnvironment,
  conversationId: string, requestId: string): Promise<void> {
  const owner = capturedEnvironment(environment);
  await freezeCorePreviewOwnerIntent(owner.nativeAgentPreview, owner.storage, conversationId, requestId);
}

/** The original startup barrier stays attached to the captured owner even if
 * another runtime has since been selected. Structural copies carry no owner. */
export function ensureRuntimeStructuredResources(environment: RuntimeEnvironment): Promise<StructuredStartupImportResult> {
  return ensureCapturedEnvironment(capturedEnvironment(environment));
}
