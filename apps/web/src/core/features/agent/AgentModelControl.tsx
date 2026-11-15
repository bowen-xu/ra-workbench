import type { LlmConfiguration, LlmModelDescriptor, LlmModelSelection } from "@research-assistant/core/ai";
import { useEffect, useMemo, useState } from "react";

import {
  aiAssistanceErrorMessage,
  getAiAssistanceGateway,
  type AiAssistanceGateway,
} from "../../services/aiAssistanceGateway";

export type AgentModelGateway = Pick<AiAssistanceGateway, "getLlmConfiguration" | "fetchLlmProviderModels">;
import { AgentModelPicker } from "./AgentModelPicker";
import { useAgentRequestGuard } from "./useAgentMountedGuard";
import { cacheLlmModels, readCachedLlmModels } from "../models/llmModelCatalogCache";
import { preferredSelection, providerReady, resolvedSelection, selectionReadyForKind } from "../models/llmRouteSelection";
import { LLM_CONFIGURATION_CHANGED_EVENT } from "../models/llmConfigurationEvents";

export interface AgentModelControlProps {
  readonly route: string;
  readonly label: string;
  readonly disabled: boolean;
  readonly gateway?: AgentModelGateway;
  readonly modelKind?: "text" | "vision";
  readonly selection?: LlmModelSelection | undefined;
  readonly onSelection: (selection: LlmModelSelection | undefined) => void;
}

export function AgentModelControl({
  disabled, route, label: accessibleLabel,
  gateway: providedGateway,
  modelKind = "text",
  selection: providedSelection,
  onSelection,
}: AgentModelControlProps): React.JSX.Element {
  const defaultGateway = useMemo(() => getAiAssistanceGateway(), []);
  const gateway = providedGateway ?? defaultGateway;
  const [configuration, setConfiguration] = useState<LlmConfiguration | undefined>(undefined);
  const [internalSelection, setInternalSelection] = useState<LlmModelSelection | undefined>(undefined);
  const [models, setModels] = useState<readonly LlmModelDescriptor[]>([]);
  const [catalogRevision, setCatalogRevision] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | undefined>(undefined);
  const beginConfigurationRequest = useAgentRequestGuard();
  const beginModelsRequest = useAgentRequestGuard();

  useEffect(() => {
    const load = (): void => {
      const active = beginConfigurationRequest();
      void gateway.getLlmConfiguration()
        .then(async (next) => {
          if (!active()) return;
          setConfiguration(next);
          setCatalogRevision((revision) => revision + 1);
          const providedSelectionReady = selectionReadyForKind(next, providedSelection, modelKind);
          let nextSelection = providedSelectionReady
            ? providedSelection
            : resolvedSelection(next, route);
          if (nextSelection === undefined) {
            const provider = next.providers.find((item) => item.modelKinds.includes(modelKind) && providerReady(item));
            if (provider !== undefined) {
              const cached = readCachedLlmModels(provider.providerId);
              const result = cached ?? (await gateway.fetchLlmProviderModels(provider.providerId)).models;
              if (cached === undefined) cacheLlmModels(provider.providerId, result);
              const model = result.find((item) => item.isDefault) ?? result[0];
              if (model !== undefined) nextSelection = preferredSelection(provider.providerId, model);
            }
          }
          if (!active()) return;
          setInternalSelection(nextSelection);
          if (providedSelection === undefined || !providedSelectionReady) onSelection(nextSelection);
        })
        .catch((reason: unknown) => { if (active()) setError(aiAssistanceErrorMessage(reason)); })
        .finally(() => { if (active()) setLoading(false); });
    };
    load();
    window.addEventListener(LLM_CONFIGURATION_CHANGED_EVENT, load);
    return () => window.removeEventListener(LLM_CONFIGURATION_CHANGED_EVENT, load);
  }, [beginConfigurationRequest, gateway, modelKind, onSelection, providedSelection, route]);

  useEffect(() => {
    if (providedSelection !== undefined) setInternalSelection(providedSelection);
  }, [providedSelection]);

  const selection = providedSelection ?? internalSelection;

  useEffect(() => {
    const active = beginModelsRequest();
    if (selection === undefined) {
      setModels([]);
      return;
    }
    const cached = readCachedLlmModels(selection.providerId);
    if (cached !== undefined) setModels(cached);
    void gateway.fetchLlmProviderModels(selection.providerId)
      .then((result) => {
        if (!active() || result.providerId !== selection.providerId) return;
        cacheLlmModels(result.providerId, result.models);
        setModels(result.models);
      })
      .catch((reason: unknown) => {
        if (active() && cached === undefined) setError(aiAssistanceErrorMessage(reason));
      });
  }, [beginModelsRequest, catalogRevision, gateway, selection?.providerId]);

  const provider = configuration?.providers.find((item) => item.providerId === selection?.providerId);
  const selectedModel = models.find((item) => item.id === selection?.modelId);
  const efforts = selectedModel?.supportedReasoningEfforts ?? [];
  const selectedEffort = selection?.reasoningEffort ?? selectedModel?.defaultReasoningEffort ?? efforts[0];
  const modelLabel = selection === undefined
    ? error ?? (loading ? "正在读取模型…" : "未配置文本模型")
    : selectedModel?.displayName ?? selection.modelId;
  const fullLabel = selection === undefined
    ? modelLabel
    : `${provider?.displayName ?? selection.providerId} · ${modelLabel}`;
  const defaultSelection = configuration === undefined ? undefined : resolvedSelection(configuration, route);
  const defaultModel = models.find(model => model.id === defaultSelection?.modelId);
  const defaultEffort = defaultSelection?.reasoningEffort ?? defaultModel?.defaultReasoningEffort ?? defaultModel?.supportedReasoningEfforts[0];
  const isDefault = selection?.providerId === defaultSelection?.providerId
    && selection?.modelId === defaultSelection?.modelId && selectedEffort === defaultEffort;

  useEffect(() => {
    if (configuration === undefined || selection === undefined || loading || models.length === 0
      || models.some((model) => model.id === selection.modelId)) return;
    const fallback = models.find((model) => model.isDefault) ?? models[0];
    if (fallback === undefined) return;
    const next = preferredSelection(selection.providerId, fallback);
    setInternalSelection(next);
    onSelection(next);
  }, [configuration, loading, models, onSelection, selection]);

  function choose(next: LlmModelSelection): void {
    setInternalSelection(next);
    onSelection(next);
  }

  return <AgentModelPicker
    disabled={disabled || selection === undefined}
    label={accessibleLabel} modelLabel={modelLabel} title={error ?? fullLabel}
    modelId={selection?.modelId} models={models} efforts={efforts} effort={selectedEffort}
    openAi={selection?.providerId === "openai"} fastMode={selection?.serviceTier === "fast"}
    supportsFastMode={selectedModel?.supportsFastMode === true}
    isDefault={isDefault} canUseDefault={defaultSelection !== undefined}
    onDefault={() => { if (defaultSelection) choose(defaultSelection); }}
    onReset={() => {
      if (!defaultSelection) return;
      const { providerId, modelId, reasoningEffort } = defaultSelection;
      choose({ providerId, modelId,
        ...(reasoningEffort === undefined ? {} : { reasoningEffort }),
        ...(selection?.serviceTier === undefined ? {} : { serviceTier: selection.serviceTier }) });
    }}
    onModel={(modelId) => {
      if (!selection) return;
      const model = models.find((item) => item.id === modelId);
      if (model) choose(preferredSelection(selection.providerId, model));
    }}
    onEffort={(reasoningEffort) => { if (selection) choose({ ...selection, reasoningEffort }); }}
    onFastMode={(fastMode) => { if (selection) choose({ ...selection, serviceTier: fastMode ? "fast" : "default" }); }}
  />;
}
