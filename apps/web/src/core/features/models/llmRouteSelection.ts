import type {
  LlmConfiguration,
  LlmModelDescriptor,
  LlmModelKind,
  LlmModelSelection,
  LlmProviderConfig,
  LlmProviderId,
} from "@research-assistant/core/ai";

export function routeModelKind(configuration: LlmConfiguration, taskKind: string): LlmModelKind {
  return configuration.useCases.find((route) => route.routeId === taskKind)?.modelKind ?? "text";
}

export function routeLabel(configuration: LlmConfiguration | undefined, taskKind: string): string {
  return configuration?.useCases.find((route) => route.routeId === taskKind)?.label ?? "本功能";
}

export function preferredSelection(
  providerId: LlmProviderId,
  model: LlmModelDescriptor,
): LlmModelSelection {
  return {
    providerId,
    modelId: model.id,
    ...(model.defaultReasoningEffort === undefined
      ? {} : { reasoningEffort: model.defaultReasoningEffort }),
  };
}

export function providerReady(provider: LlmProviderConfig): boolean {
  return provider.enabled && (provider.authMode !== "api-key" || provider.credentialConfigured);
}

export function providerOptionLabel(provider: LlmProviderConfig): string {
  if (!provider.enabled) return `${provider.displayName}（未启用）`;
  if (provider.authMode === "api-key" && !provider.credentialConfigured) {
    return `${provider.displayName}（缺少 API key）`;
  }
  return provider.displayName;
}

export function modelLikelyBelongsToProvider(providerId: LlmProviderId, modelId: string): boolean {
  const id = modelId.toLowerCase();
  if (providerId !== "openai") return true;
  return !id.includes("qwen") && !id.includes("qvq") && !id.includes("deepseek");
}

export function selectionReadyForKind(
  configuration: LlmConfiguration,
  selection: LlmModelSelection | undefined,
  modelKind: LlmModelKind,
): selection is LlmModelSelection {
  if (selection === undefined || !modelLikelyBelongsToProvider(selection.providerId, selection.modelId)) {
    return false;
  }
  const provider = configuration.providers.find((item) => item.providerId === selection.providerId);
  return provider !== undefined && providerReady(provider) && provider.modelKinds.includes(modelKind);
}

export function resolvedSelection(
  configuration: LlmConfiguration,
  taskKind: string,
): LlmModelSelection | undefined {
  const modelKind = routeModelKind(configuration, taskKind);
  const candidates = [
    configuration.useCases.find((route) => route.routeId === taskKind)?.selection,
    configuration.defaultModels.find((route) => route.routeId === modelKind)?.selection,
    ...(modelKind === "text"
      ? [configuration.defaultModels.find((route) => route.routeId === "text")?.selection]
      : []),
  ];
  return candidates.find((selection) => selectionReadyForKind(configuration, selection, modelKind));
}
