import { notify } from "../../services/notifications";
import { useCallback, useEffect, useMemo, useState } from "react";
import type {
  LlmConfiguration,
  LlmModelDescriptor,
  LlmModelSelection,
  LlmProviderId,
} from "@research-assistant/core/ai";

import {
  aiAssistanceErrorMessage,
  getAiAssistanceGateway,
  type AiAssistanceGateway,
} from "../../services/aiAssistanceGateway";
import { cacheLlmModels, readCachedLlmModels } from "./llmModelCatalogCache";
import {
  modelLikelyBelongsToProvider,
  preferredSelection,
  providerOptionLabel,
  providerReady,
  resolvedSelection,
  routeLabel,
  routeModelKind,
} from "./llmRouteSelection";

import { LLM_CONFIGURATION_CHANGED_EVENT, announceLlmConfigurationChanged } from "./llmConfigurationEvents";

export type LlmRouteGateway = Pick<AiAssistanceGateway, "getLlmConfiguration" | "fetchLlmProviderModels" | "saveLlmModelRoutes">;

export interface LlmRoutePickerProps {
  readonly disabled?: boolean;
  readonly description?: string;
  readonly gateway?: LlmRouteGateway;
  readonly legend?: string;
  readonly onSelection: (selection: LlmModelSelection | undefined) => void;
  readonly taskKind: string;
}

export function LlmRoutePicker({
  disabled = false,
  description,
  gateway: providedGateway,
  legend,
  onSelection,
  taskKind,
}: LlmRoutePickerProps): React.JSX.Element {
  const defaultGateway = useMemo(() => getAiAssistanceGateway(), []);
  const gateway = providedGateway ?? defaultGateway;
  const [configuration, setConfiguration] = useState<LlmConfiguration>();
  const [selection, setSelection] = useState<LlmModelSelection>();
  const [models, setModels] = useState<readonly LlmModelDescriptor[]>([]);
  const [onlyThisRun, setOnlyThisRun] = useState(false);
  const [saving, setSaving] = useState(false);
  const [autoSelectionAttempted, setAutoSelectionAttempted] = useState(false);

  const refresh = useCallback(async () => {
    const next = await gateway.getLlmConfiguration();
    setConfiguration(next);
    setSelection(resolvedSelection(next, taskKind));
  }, [gateway, taskKind]);

  useEffect(() => {
    void refresh().catch((reason: unknown) => notify.error(aiAssistanceErrorMessage(reason)));
    const listener = (): void => { void refresh().catch(() => undefined); };
    window.addEventListener(LLM_CONFIGURATION_CHANGED_EVENT, listener);
    return () => window.removeEventListener(LLM_CONFIGURATION_CHANGED_EVENT, listener);
  }, [refresh]);

  useEffect(() => {
    if (selection === undefined) {
      setModels([]);
      return;
    }
    let active = true;
    setModels([]);
    const cached = readCachedLlmModels(selection.providerId);
    if (cached !== undefined) {
      setModels(cached);
      return () => { active = false; };
    }
    void gateway.fetchLlmProviderModels(selection.providerId)
      .then((result) => {
        if (!active || result.providerId !== selection.providerId) return;
        cacheLlmModels(result.providerId, result.models);
        setModels(result.models);
      })
      .catch(() => { if (active) setModels([]); });
    return () => { active = false; };
  }, [gateway, selection?.providerId]);

  useEffect(() => {
    if (selection === undefined || models.length === 0) return;
    if (modelLikelyBelongsToProvider(selection.providerId, selection.modelId)) return;
    const preferred = models.find((model) => model.isDefault) ?? models[0];
    if (preferred === undefined) return;
    void apply(preferredSelection(selection.providerId, preferred));
  }, [models, selection?.modelId, selection?.providerId]);

  useEffect(() => { onSelection(onlyThisRun ? selection : undefined); }, [onSelection, onlyThisRun, selection]);

  const modelKind = configuration === undefined ? "text" : routeModelKind(configuration, taskKind);
  const routeName = legend ?? `${routeLabel(configuration, taskKind)}模型`;
  const providers = configuration?.providers.filter((provider) =>
    provider.modelKinds.includes(modelKind)) ?? [];

  useEffect(() => { setAutoSelectionAttempted(false); }, [taskKind]);

  useEffect(() => {
    if (configuration === undefined || selection !== undefined || saving || autoSelectionAttempted) return;
    const provider = providers.find(providerReady);
    if (provider === undefined) return;
    setAutoSelectionAttempted(true);
    void selectProvider(provider.providerId);
  }, [autoSelectionAttempted, configuration, providers, saving, selection]);

  async function apply(next: LlmModelSelection): Promise<void> {
    setSelection(next);

    if (onlyThisRun) return;
    setSaving(true);
    try {
      const updated = await gateway.saveLlmModelRoutes([{
        routeKind: "use_case",
        routeId: taskKind,
        selection: next,
      }]);
      setConfiguration(updated);
      announceLlmConfigurationChanged();
    } catch (reason: unknown) {
      notify.error(aiAssistanceErrorMessage(reason));
      await refresh().catch(() => undefined);
    } finally {
      setSaving(false);
    }
  }

  async function inheritDefault(): Promise<void> {
    setSaving(true);
    try {
      const updated = await gateway.saveLlmModelRoutes([{
        routeKind: "use_case",
        routeId: taskKind,
      }]);
      setConfiguration(updated);
      setSelection(resolvedSelection(updated, taskKind));
      setAutoSelectionAttempted(false);
      announceLlmConfigurationChanged();
    } catch (reason: unknown) {
      notify.error(aiAssistanceErrorMessage(reason));
    } finally {
      setSaving(false);
    }
  }

  async function selectProvider(providerId: LlmProviderId): Promise<void> {
    setSaving(true);

    setSelection(undefined);
    setModels([]);
    try {
      const provider = configuration?.providers.find((item) => item.providerId === providerId);
      if (provider !== undefined && !providerReady(provider)) {
        throw new Error(`${provider.displayName} 尚未启用，请先在 AI 设置中启用并配置凭据。`);
      }
      const cached = readCachedLlmModels(providerId);
      const nextModels = cached ?? (await gateway.fetchLlmProviderModels(providerId)).models;
      if (cached === undefined) cacheLlmModels(providerId, nextModels);
      setModels(nextModels);
      const preferred = nextModels.find((model) => model.isDefault) ?? nextModels[0];
      if (preferred === undefined) throw new Error("该模型服务没有可用模型，请先在 AI 设置中检查账号或模型目录。");
      await apply(preferredSelection(providerId, preferred));
    } catch (reason: unknown) {
      notify.error(aiAssistanceErrorMessage(reason));
    } finally {
      setSaving(false);
    }
  }

  const selectedModel = models.find((model) => model.id === selection?.modelId);
  const displayedModelId = selection !== undefined
    && modelLikelyBelongsToProvider(selection.providerId, selection.modelId) ? selection.modelId : "";
  const efforts = selectedModel?.supportedReasoningEfforts
    ?? (selection?.providerId === "openai" ? ["low", "medium", "high", "xhigh"] as const : []);

  return <fieldset className="llm-route-picker" disabled={disabled || saving}>
    <legend>{routeName}</legend>
    {description !== undefined && <p className="llm-route-picker-description">{description}</p>}
    <label>模型服务
      <select value={selection?.providerId ?? ""} onChange={(event) => void selectProvider(event.target.value as LlmProviderId)}>
        <option disabled value="">请先在 AI 设置中配置</option>
        {providers.map((provider) => <option disabled={!providerReady(provider)} key={provider.providerId} value={provider.providerId}>{providerOptionLabel(provider)}</option>)}
      </select>
    </label>
    <label>模型
      <input list={`local-models-${taskKind}`} onBlur={(event) => {
        if (selection !== undefined && event.target.value.trim() !== "") void apply({ ...selection, modelId: event.target.value.trim() });
      }} onChange={(event) => setSelection((current) => current === undefined ? current : { ...current, modelId: event.target.value })} value={displayedModelId} />
      <datalist id={`local-models-${taskKind}`}>{models.map((model) => <option key={model.id} value={model.id} />)}</datalist>
    </label>
    {selection?.providerId === "openai" && <label>推理强度
      <select onChange={(event) => void apply({ ...selection, reasoningEffort: event.target.value as NonNullable<LlmModelSelection["reasoningEffort"]> })} value={selection.reasoningEffort ?? selectedModel?.defaultReasoningEffort ?? "medium"}>
        {efforts.map((effort) => <option key={effort} value={effort}>{effort}</option>)}
      </select>
    </label>}
    <label className="llm-route-picker-run"><input checked={onlyThisRun} onChange={(event) => setOnlyThisRun(event.target.checked)} type="checkbox" />仅本次运行</label>
    {!onlyThisRun && <button disabled={saving} onClick={() => void inheritDefault()} type="button">恢复继承全局{modelKind === "vision" ? "视觉" : "文本"}模型</button>}
    <small>{onlyThisRun ? "本次选择不会写入设置。" : `修改后会保存为“${routeName}”的功能默认模型。`}</small>
  </fieldset>;
}
