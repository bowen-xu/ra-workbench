import type {AgentCatalogGateway} from "@research-assistant/data/agent";
import {getAppRuntime, type AppRuntime} from "../../../core/runtime/appRuntime";
import {invalidateConversationCatalog} from "../../../core/runtime/conversationCatalogEvents";
import {announceLlmConfigurationChanged} from "../../../core/public/modelEvents";

/** Synthetic observations for launcher and focus tests. The normal App never
 * imports this fixture, and callers receive no runtime or gateway objects. */
function previewRuntime(): AppRuntime {
  const runtime = getAppRuntime();
  if (runtime.mode !== "preview" || !runtime.preview) {
    throw new Error("BROWSER_FIXTURE_PREVIEW_REQUIRED");
  }
  return runtime;
}

let heldPreparation: {runtime: AppRuntime; fail: () => void} | undefined;

export function holdLauncherPreparation(): void {
  const runtime = previewRuntime();
  const preparation = runtime.gateways.agentContextGateway?.preparation;
  if (!preparation) throw new Error("BROWSER_FIXTURE_PREPARATION_REQUIRED");
  heldPreparation = undefined;
  preparation.proven.prepare = async () => new Promise<never>((_resolve, reject) => {
    heldPreparation = {runtime, fail: () => reject(new Error("LAUNCHER_PREPARATION_FAILED"))};
  });
}

export function launcherPreparationReached(): boolean {
  return heldPreparation?.runtime === previewRuntime();
}

export function failHeldLauncherPreparation(): void {
  if (heldPreparation?.runtime !== previewRuntime()) {
    throw new Error("BROWSER_FIXTURE_PREPARATION_NOT_HELD");
  }
  heldPreparation.fail();
  heldPreparation = undefined;
}

export function refreshLauncherActivity(): void {
  previewRuntime();
  invalidateConversationCatalog();
}

let activityCatalog: AgentCatalogGateway | undefined;
let internalActivity = false;

export function setLauncherInternalActivity(active: boolean): void {
  const catalog = previewRuntime().gateways.agentCatalogGateway;
  if (!catalog) throw new Error("BROWSER_FIXTURE_CATALOG_REQUIRED");
  if (catalog !== activityCatalog) {
    activityCatalog = catalog;
    catalog.activity = async () => internalActivity ? [{id: "off-page-internal", internal: true}] : [];
  }
  internalActivity = active;
  invalidateConversationCatalog();
}

export async function selectLauncherOpenAiModel(): Promise<void> {
  const gateway = previewRuntime().gateways.getAiAssistanceGateway;
  if (!gateway) throw new Error("BROWSER_FIXTURE_MODEL_REQUIRED");
  await gateway.saveLlmModelRoutes([{routeKind: "default", routeId: "text",
    selection: {providerId: "openai", modelId: "gpt-5.6-terra", reasoningEffort: "medium"}}]);
  announceLlmConfigurationChanged();
}
