import { ControlHint } from "@research-assistant/workbench-sdk/overlay";
import { OverlayPortal } from "@research-assistant/workbench-sdk/overlay";
import { useEffect, useRef, useState } from "react";
import { Check, ChevronDown, ChevronRight, RotateCcw } from "lucide-react";
import type {
  LlmConfiguration,
  LlmModelDescriptor,
  LlmModelSelection,
  LlmProviderId,
  PaperAiReadingTurn,
} from "@research-assistant/core/ai";

import type { AiAssistanceGateway } from "@research-assistant/workbench-sdk/ai-assistance";
import { aiAssistanceErrorMessage } from "@research-assistant/workbench-sdk/ai-assistance";
import {
  LLM_CONFIGURATION_CHANGED_EVENT,
  announceLlmConfigurationChanged,
} from "../../../../../../apps/web/src/core/public/modelEvents";
import { cacheLlmModels, readCachedLlmModels } from "../../../../../../apps/web/src/core/public/modelCatalog";
import {
  preferredSelection,
  providerOptionLabel,
  providerReady,
  resolvedSelection,
  selectionReadyForKind,
} from "../../../../../../apps/web/src/core/public/modelSelection";
import { useAgentRequestGuard } from "../../../../../../apps/web/src/core/public/agentPresentation/useAgentMountedGuard";

const ROUTE_ID = "paper_reading_chat";

interface ModelControlProps {
  readonly disabled: boolean;
  readonly gateway: AiAssistanceGateway;
  readonly onSelection: (selection: LlmModelSelection | undefined) => void;
  readonly selection: LlmModelSelection | undefined;
}

interface PopoverPosition {
  readonly bottom: number;
  readonly left: number;
  readonly maxHeight: number;
  readonly width: number;
}

interface SubmenuPosition {
  readonly left: number;
  readonly maxHeight: number;
  readonly top: number;
  readonly width: number;
}

type ModelSubmenu = "provider" | "model" | "effort";

function anchoredPosition(anchor: HTMLElement, preferredWidth: number): PopoverPosition {
  const rect = anchor.getBoundingClientRect();
  const viewportWidth = document.documentElement.clientWidth || window.innerWidth;
  const viewportHeight = document.documentElement.clientHeight || window.innerHeight;
  const width = Math.min(preferredWidth, viewportWidth - 16);
  return {
    bottom: Math.max(8, viewportHeight - rect.top + 8),
    left: Math.max(8, Math.min(rect.right - width, viewportWidth - width - 8)),
    maxHeight: Math.max(180, rect.top - 24),
    width,
  };
}

function adjacentPosition(anchor: HTMLElement, preferredWidth: number): SubmenuPosition {
  const rect = anchor.getBoundingClientRect();
  const viewportWidth = document.documentElement.clientWidth || window.innerWidth;
  const viewportHeight = document.documentElement.clientHeight || window.innerHeight;
  const width = Math.min(preferredWidth, viewportWidth - 16);
  const right = rect.right + 8;
  const left = right + width <= viewportWidth - 8
    ? right
    : Math.max(8, rect.left - width - 8);
  const top = Math.max(8, Math.min(rect.top, viewportHeight - 156));
  return {
    left,
    maxHeight: Math.max(140, viewportHeight - top - 8),
    top,
    width,
  };
}

function sameSelection(left: LlmModelSelection | undefined, right: LlmModelSelection | undefined): boolean {
  return left?.providerId === right?.providerId
    && left?.modelId === right?.modelId
    && left?.reasoningEffort === right?.reasoningEffort;
}

export function PaperAiReadingModelControl({
  disabled,
  gateway,
  onSelection,
  selection,
}: ModelControlProps): React.JSX.Element {
  const triggerRef = useRef<HTMLButtonElement>(null);
  const popoverRef = useRef<HTMLDivElement>(null);
  const submenuRef = useRef<HTMLDivElement>(null);
  const selectionRef = useRef(selection);
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState<PopoverPosition>();
  const [submenu, setSubmenu] = useState<ModelSubmenu>();
  const [submenuPosition, setSubmenuPosition] = useState<SubmenuPosition>();
  const [configuration, setConfiguration] = useState<LlmConfiguration>();
  const [models, setModels] = useState<readonly LlmModelDescriptor[]>([]);
  const [loadingModels, setLoadingModels] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string>();
  const beginConfigurationRequest = useAgentRequestGuard();
  const beginModelsRequest = useAgentRequestGuard();

  useEffect(() => { selectionRef.current = selection; }, [selection]);

  async function refreshConfiguration(active: () => boolean): Promise<void> {
    const next = await gateway.getLlmConfiguration();
    if (!active()) return;
    setConfiguration(next);
    if (selectionRef.current === undefined && active()) onSelection(resolvedSelection(next, ROUTE_ID));
  }

  useEffect(() => {
    const active = beginConfigurationRequest();
    void refreshConfiguration(active).catch((reason: unknown) => {
      if (active()) setError(aiAssistanceErrorMessage(reason));
    });
    const listener = (): void => {
      const current = beginConfigurationRequest();
      void refreshConfiguration(current).catch((reason: unknown) => {
        if (current()) setError(aiAssistanceErrorMessage(reason));
      });
    };
    window.addEventListener(LLM_CONFIGURATION_CHANGED_EVENT, listener);
    return () => window.removeEventListener(LLM_CONFIGURATION_CHANGED_EVENT, listener);
  }, [beginConfigurationRequest, gateway]);

  useEffect(() => {
    if (configuration === undefined || selection === undefined
      || selectionReadyForKind(configuration, selection, "vision")) return;
    const fallback = resolvedSelection(configuration, ROUTE_ID);
    if (fallback === undefined || sameSelection(fallback, selection)) return;
    setError("该会话最近使用的模型当前不可用，已切换到可用的文献阅读默认模型。");
    onSelection(fallback);
  }, [configuration, onSelection, selection]);

  useEffect(() => {
    const active = beginModelsRequest();
    if (selection === undefined) {
      setModels([]);
      return;
    }
    const cached = readCachedLlmModels(selection.providerId);
    if (cached !== undefined) setModels(cached);
    setLoadingModels(cached === undefined);
    void gateway.fetchLlmProviderModels(selection.providerId)
      .then((result) => {
        if (!active() || result.providerId !== selection.providerId) return;
        cacheLlmModels(result.providerId, result.models);
        setModels(result.models);
      })
      .catch((reason: unknown) => {
        if (active() && cached === undefined) setError(aiAssistanceErrorMessage(reason));
      })
      .finally(() => { if (active()) setLoadingModels(false); });
  }, [beginModelsRequest, gateway, selection?.providerId]);

  useEffect(() => {
    if (loadingModels || selection === undefined || models.length === 0
      || models.some((model) => model.id === selection.modelId)) return;
    const fallback = models.find((model) => model.isDefault) ?? models[0];
    if (fallback === undefined) return;
    setError("该会话最近使用的模型已不在目录中，已切换到当前 Provider 的默认模型。");
    onSelection(preferredSelection(selection.providerId, fallback));
  }, [loadingModels, models, onSelection, selection]);

  useEffect(() => {
    if (!open) return;
    const update = (): void => {
      if (triggerRef.current !== null) setPosition(anchoredPosition(triggerRef.current, 330));
    };
    const dismiss = (event: PointerEvent): void => {
      const target = event.target;
      if (!(target instanceof Node)
        || triggerRef.current?.contains(target)
        || popoverRef.current?.contains(target)
        || submenuRef.current?.contains(target)) return;
      setOpen(false);
      setSubmenu(undefined);
    };
    const keydown = (event: KeyboardEvent): void => {
      if (event.key !== "Escape") return;
      if (submenuRef.current !== null) {
        setSubmenu(undefined);
        return;
      }
      setOpen(false);
      window.requestAnimationFrame(() => triggerRef.current?.focus({ preventScroll: true }));
    };
    update();
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("keydown", keydown);
    return () => {
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
      document.removeEventListener("pointerdown", dismiss);
      document.removeEventListener("keydown", keydown);
    };
  }, [open]);

  const providers = configuration?.providers.filter((provider) => provider.modelKinds.includes("vision")) ?? [];
  const provider = providers.find((item) => item.providerId === selection?.providerId);
  const selectedModel = models.find((model) => model.id === selection?.modelId);
  const efforts = selectedModel?.supportedReasoningEfforts ?? [];
  const featureDefault = configuration?.useCases.find((route) => route.routeId === ROUTE_ID)?.selection;
  const isFeatureDefault = sameSelection(featureDefault, selection);
  const modelLabel = selectedModel?.displayName || selection?.modelId || "选择模型";
  const compactLabel = selection === undefined
    ? "未配置视觉模型"
    : [provider?.displayName ?? selection.providerId, modelLabel, selection.reasoningEffort]
      .filter((value) => value !== undefined && value !== "")
      .join(" · ");

  async function loadProviderModels(providerId: LlmProviderId): Promise<readonly LlmModelDescriptor[]> {
    const cached = readCachedLlmModels(providerId);
    if (cached !== undefined) return cached;
    const result = await gateway.fetchLlmProviderModels(providerId);
    cacheLlmModels(result.providerId, result.models);
    return result.models;
  }

  async function selectProvider(providerId: LlmProviderId): Promise<void> {
    const nextProvider = providers.find((item) => item.providerId === providerId);
    if (nextProvider === undefined || !providerReady(nextProvider)) return;
    setError(undefined);
    setLoadingModels(true);
    try {
      const nextModels = await loadProviderModels(providerId);
      setModels(nextModels);
      const preferred = nextModels.find((model) => model.isDefault) ?? nextModels[0];
      if (preferred === undefined) throw new Error("该 Provider 没有可用模型。");
      onSelection(preferredSelection(providerId, preferred));
      setSubmenu(undefined);
    } catch (reason: unknown) {
      setError(aiAssistanceErrorMessage(reason));
    } finally {
      setLoadingModels(false);
    }
  }

  function openSubmenu(kind: ModelSubmenu, anchor: HTMLElement): void {
    if (saving || (kind === "model" && (loadingModels || selection === undefined))) return;
    setSubmenu(kind);
    setSubmenuPosition(adjacentPosition(anchor, kind === "model" ? 300 : 240));
  }

  async function setAsDefault(): Promise<void> {
    if (selection === undefined || isFeatureDefault) return;
    setSaving(true);
    setError(undefined);
    try {
      const updated = await gateway.saveLlmModelRoutes([{
        routeKind: "use_case",
        routeId: ROUTE_ID,
        selection,
      }]);
      setConfiguration(updated);
      announceLlmConfigurationChanged();
    } catch (reason: unknown) {
      setError(aiAssistanceErrorMessage(reason));
    } finally {
      setSaving(false);
    }
  }

  async function inheritVisionDefault(): Promise<void> {
    setSaving(true);
    setError(undefined);
    try {
      const updated = await gateway.saveLlmModelRoutes([{
        routeKind: "use_case",
        routeId: ROUTE_ID,
      }]);
      const next = resolvedSelection(updated, ROUTE_ID);
      setConfiguration(updated);
      onSelection(next);
      announceLlmConfigurationChanged();
    } catch (reason: unknown) {
      setError(aiAssistanceErrorMessage(reason));
    } finally {
      setSaving(false);
    }
  }

  return <>
    <ControlHint content={compactLabel}><button
      aria-expanded={open}
      aria-haspopup="dialog"
      aria-label="选择 AI 文献阅读模型"
      className="paper-ai-model-trigger"
      disabled={disabled}
      onClick={() => setOpen((current) => {
        if (current) setSubmenu(undefined);
        return !current;
      })}
      ref={triggerRef}
      type="button"
    >
      <span>{compactLabel}</span>
      <ChevronDown aria-hidden="true" />
    </button></ControlHint>
    {open && position !== undefined && <OverlayPortal><>
      <div
        aria-label="AI 文献阅读模型"
        className="paper-ai-model-popover"
        ref={popoverRef}
        role="dialog"
        style={position}
      >
        <div className="paper-ai-model-menu-rows">
          <button aria-expanded={submenu === "provider"} aria-label="选择 Provider" disabled={saving} onClick={(event) => openSubmenu("provider", event.currentTarget)} type="button">
            <span>Provider</span>
            <strong>{provider?.displayName ?? selection?.providerId ?? "未配置"}</strong>
            <ChevronRight aria-hidden="true" />
          </button>
          <button aria-expanded={submenu === "model"} aria-label="选择模型" disabled={saving || loadingModels || selection === undefined} onClick={(event) => openSubmenu("model", event.currentTarget)} type="button">
            <span>Model</span>
            <strong>{loadingModels ? "读取中" : modelLabel}</strong>
            <ChevronRight aria-hidden="true" />
          </button>
          {selection !== undefined && efforts.length > 0 && <button aria-expanded={submenu === "effort"} aria-label="选择推理强度" disabled={saving} onClick={(event) => openSubmenu("effort", event.currentTarget)} type="button">
            <span>Effort</span>
            <strong>{selection.reasoningEffort ?? selectedModel?.defaultReasoningEffort ?? efforts[0]}</strong>
            <ChevronRight aria-hidden="true" />
          </button>}
        </div>
        <div className="paper-ai-model-popover-actions">
          <button disabled={saving || selection === undefined || isFeatureDefault} onClick={() => void setAsDefault()} type="button">
            <Check aria-hidden="true" />
            {isFeatureDefault ? "当前为功能默认" : "设为文献阅读默认"}
          </button>
          <button disabled={saving} onClick={() => void inheritVisionDefault()} type="button">
            <RotateCcw aria-hidden="true" />
            恢复全局视觉默认
          </button>
        </div>
        <small>当前选择只用于本会话后续发送；仅点击“设为默认”时写入功能配置。</small>
        {error !== undefined && <p className="paper-ai-model-error">{error}</p>}
      </div>
      {submenu !== undefined && submenuPosition !== undefined && <div
        aria-label={`${submenu} 选项`}
        className="paper-ai-model-submenu"
        ref={submenuRef}
        role="menu"
        style={submenuPosition}
      >
        {submenu === "provider" && providers.map((item) => <button
          disabled={!providerReady(item)}
          key={item.providerId}
          onClick={() => void selectProvider(item.providerId)}
          role="menuitemradio"
          type="button"
        >
          <span>{providerOptionLabel(item)}</span>
          {item.providerId === selection?.providerId && <Check aria-hidden="true" />}
        </button>)}
        {submenu === "model" && models.map((model) => <button
          aria-label={`选择模型 ${model.displayName}`}
          key={model.id}
          onClick={() => {
            if (selection === undefined) return;
            onSelection(preferredSelection(selection.providerId, model));
            setSubmenu(undefined);
          }}
          role="menuitemradio"
          type="button"
        >
          <span>{model.displayName}</span>
          {model.id === selection?.modelId && <Check aria-hidden="true" />}
        </button>)}
        {submenu === "effort" && efforts.map((effort) => <button
          aria-label={`选择推理强度 ${effort}`}
          key={effort}
          onClick={() => {
            if (selection === undefined) return;
            onSelection({ ...selection, reasoningEffort: effort });
            setSubmenu(undefined);
          }}
          role="menuitemradio"
          type="button"
        >
          <span>{effort}</span>
          {effort === selection?.reasoningEffort && <Check aria-hidden="true" />}
        </button>)}
      </div>}
      </></OverlayPortal>}
  </>;
}

interface ContextIndicatorProps {
  readonly turn: PaperAiReadingTurn | undefined;
}

function formatTokens(value: number): string {
  return new Intl.NumberFormat("zh-CN").format(value);
}

export function PaperAiReadingContextIndicator({ turn }: ContextIndicatorProps): React.JSX.Element {
  const triggerRef = useRef<HTMLButtonElement>(null);
  const popoverRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState<PopoverPosition>();
  const input = turn?.inputTokens;
  const limit = turn?.contextWindowTokens;
  const percentage = input === undefined || limit === undefined
    ? undefined : Math.max(0, Math.round((input / limit) * 100));
  const displayPercentage = Math.min(100, percentage ?? 0);
  const level = percentage !== undefined && percentage >= 95
    ? "danger" : percentage !== undefined && percentage >= 80 ? "warning" : "normal";
  const title = turn === undefined || input === undefined
    ? "暂无真实上下文用量"
    : limit === undefined
      ? `上一轮输入 ${formatTokens(input)} tokens，上下文上限未知`
      : `上一轮输入 ${formatTokens(input)} / ${formatTokens(limit)} tokens（${percentage}%）`;

  useEffect(() => {
    if (!open) return;
    const update = (): void => {
      if (triggerRef.current !== null) setPosition(anchoredPosition(triggerRef.current, 260));
    };
    const dismiss = (event: PointerEvent): void => {
      const target = event.target;
      if (!(target instanceof Node)
        || triggerRef.current?.contains(target)
        || popoverRef.current?.contains(target)) return;
      setOpen(false);
    };
    const keydown = (event: KeyboardEvent): void => {
      if (event.key !== "Escape") return;
      setOpen(false);
      triggerRef.current?.focus();
    };
    update();
    window.addEventListener("resize", update);
    window.addEventListener("scroll", update, true);
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("keydown", keydown);
    return () => {
      window.removeEventListener("resize", update);
      window.removeEventListener("scroll", update, true);
      document.removeEventListener("pointerdown", dismiss);
      document.removeEventListener("keydown", keydown);
    };
  }, [open]);

  return <div className="paper-ai-context-control">
    <ControlHint content={title}><button
      aria-expanded={open}
      aria-label="查看上下文窗口用量"
      className={`paper-ai-context-trigger ${level}`}
      onClick={() => setOpen((current) => !current)}
      ref={triggerRef}
      type="button"
    >
      <span
        aria-hidden="true"
        className="paper-ai-context-ring"
        style={{ "--paper-ai-context-progress": `${displayPercentage * 3.6}deg` } as React.CSSProperties}
      />
    </button></ControlHint>
    {open && position !== undefined && <OverlayPortal><div
      aria-label="上下文窗口用量"
      className="paper-ai-context-popover"
      ref={popoverRef}
      role="dialog"
      style={position}
    >
      <strong>上下文窗口</strong>
      {turn === undefined || input === undefined
        ? <span>暂无 provider 返回的真实用量</span>
        : <>
          <span>{turn.providerId} · {turn.modelId}</span>
          {turn.reasoningEffort !== undefined && <span>推理强度：{turn.reasoningEffort}</span>}
          <span>上一轮输入：{formatTokens(input)} tokens</span>
          {limit === undefined
            ? <span>上下文上限未知</span>
            : <>
              <span>上下文上限：{formatTokens(limit)} tokens</span>
              <span>已用 {percentage}% · 剩余 {formatTokens(Math.max(0, limit - input))} tokens</span>
            </>}
        </>}
    </div></OverlayPortal>}
  </div>;
}

export function latestUsageTurn(turns: readonly PaperAiReadingTurn[]): PaperAiReadingTurn | undefined {
  return [...turns].reverse().find((turn) => turn.status === "succeeded" && turn.inputTokens !== undefined);
}
