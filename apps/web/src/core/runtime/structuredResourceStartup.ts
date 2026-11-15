import type { StructuredStartupImportResult } from "@research-assistant/core/structured-resources";
import { getStructuredResourceGateway } from "../services/structuredResourceGateway";
import { getAppRuntime, type AppRuntime } from "./appRuntime";
import { readStructuredStartupPreferences } from "./structuredStartupPreferences";

const pending = new WeakMap<AppRuntime, Promise<StructuredStartupImportResult>>();
const EMPTY: StructuredStartupImportResult = {paperIds: [], importedPaperCount: 0, importedTemplateCount: 0};

/** Share in-flight work and complete imports; deferred papers retry on the next ensure. */
export function ensureStructuredResourceStartup(runtime = getAppRuntime()): Promise<StructuredStartupImportResult> {
  if (runtime.mode !== "desktop" && !runtime.gateways.getStructuredResourceGateway) return Promise.resolve(EMPTY);
  const existing = pending.get(runtime);
  if (existing) return existing;
  const operation = Promise.resolve().then(() => {
    const draft = readStructuredStartupPreferences(runtime.storage);
    const gateway = runtime.gateways.getStructuredResourceGateway ?? getStructuredResourceGateway();
    return gateway.importStartup(draft);
  });
  pending.set(runtime, operation);
  void operation.then(result => {
    if (result.deferredPaperIds?.length && pending.get(runtime) === operation) pending.delete(runtime);
  }, () => { if (pending.get(runtime) === operation) pending.delete(runtime); });
  return operation;
}
