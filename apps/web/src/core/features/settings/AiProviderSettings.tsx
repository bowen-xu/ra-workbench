import { ControlHint } from "../../components/ControlHint";
import { SettingGroup, SettingRow, SettingsHint } from "./SettingsLayout";
import "./AiProviderSettings.css";
import { hasPlatformCapabilities } from "../../runtime/appRuntime";
import { openUrl } from "../../runtime/platformOpener";
import { Eye, EyeOff, KeyRound, RefreshCw, RotateCcw, Save, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Dialog } from "../../components/overlay/OverlayDialog";
import type {
  LlmConfiguration,
  LlmModelRoute,
  LlmModelDescriptor,
  LlmModelRouteDraft,
  LlmModelSelection,
  LlmProviderConfig,
  LlmProviderConfigDraft,
  LlmProviderId,
} from "@research-assistant/core/ai";

import {
  aiAssistanceErrorMessage,
  getAiAssistanceGateway,
  type AiAssistanceGateway,
} from "../../services/aiAssistanceGateway";
import { notify } from "../../services/notifications";
import { announceLlmConfigurationChanged } from "../models/llmConfigurationEvents";
import {CodexCatalogVersionSetting} from "./CodexCatalogVersionSetting";
import type {CodexCatalogVersions, CodexCatalogVersionGateway} from "../../services/codexCatalogVersionGateway";
import {
  cacheLlmModels,
  readAllCachedLlmModels,
  removeCachedLlmModels,
} from "../models/llmModelCatalogCache";

type ProviderAction = "idle" | "saving" | "loading-models" | "deleting";
type ModelsByProvider = Partial<Record<LlmProviderId, readonly LlmModelDescriptor[]>>;

interface ProviderEditorProps {
  readonly versionGateway?: CodexCatalogVersionGateway | undefined;
  readonly desktopAvailable: boolean;
  readonly gateway: AiAssistanceGateway;
  readonly models?: readonly LlmModelDescriptor[];
  readonly onChanged: (configuration: LlmConfiguration) => void;
  readonly onModels: (providerId: LlmProviderId, models: readonly LlmModelDescriptor[] | undefined) => void;
  readonly provider: LlmProviderConfig;
}

function ChatGptEditor({
  versionGateway,
  desktopAvailable,
  gateway,
  models,
  onChanged,
  onModels,
  provider,
}: ProviderEditorProps): React.JSX.Element {
  const [status, setStatus] = useState<Awaited<ReturnType<AiAssistanceGateway["getChatGptAccountStatus"]>>>();
  const [busy, setBusy] = useState(false);
  const [loginId, setLoginId] = useState<string>();
  const loginCanceled = useRef(false);
  const modelRequest = useRef(0);
  const [versions, setVersions] = useState<CodexCatalogVersions>();

  const refreshStatus = useCallback(async (): Promise<boolean> => {
    const next = await gateway.getChatGptAccountStatus();
    setStatus(next);
    return next.configured;
  }, [gateway]);

  useEffect(() => {
    if (!desktopAvailable) return;
    void refreshStatus().catch(() => undefined);
  }, [desktopAvailable, refreshStatus]);

  useEffect(() => {
    const revision = ++modelRequest.current;
    if (!desktopAvailable || !status?.configured) return;
    void gateway.fetchLlmProviderModels(provider.providerId)
      .then((result) => { if (modelRequest.current === revision) onModels(result.providerId, result.models); })
      .catch(() => undefined);
    return () => { ++modelRequest.current; };
  }, [desktopAvailable, gateway, onModels, provider.providerId, status?.configured, versions?.selectedVersion]);

  async function login(): Promise<void> {
    setBusy(true);
    try {
      const started = await gateway.startChatGptLogin();
      loginCanceled.current = false;
      setLoginId(started.loginId);
      await openUrl(started.authUrl);
      const deadline = Date.now() + 5 * 60_000;
      while (Date.now() < deadline) {
        await new Promise((resolve) => window.setTimeout(resolve, 1_500));
        if (loginCanceled.current) return;
        if (await refreshStatus()) {
          onChanged(await gateway.getLlmConfiguration());
          notify.success("ChatGPT 登录成功，登录状态会保存在本应用中。");
          return;
        }
      }
      await gateway.cancelChatGptLogin(started.loginId);
      notify.warning("登录等待已超时，可以重新发起登录。");
    } catch (reason: unknown) {
      notify.error(aiAssistanceErrorMessage(reason));
    } finally {
      setLoginId(undefined);
      setBusy(false);
    }
  }

  async function cancelLogin(): Promise<void> {
    if (loginId === undefined) return;
    loginCanceled.current = true;
    try {
      await gateway.cancelChatGptLogin(loginId);
      notify.success("已取消本次登录。");
    } catch (reason: unknown) {
      notify.error(aiAssistanceErrorMessage(reason));
    } finally {
      setLoginId(undefined);
      setBusy(false);
    }
  }

  async function logout(): Promise<void> {
    if (!window.confirm("确定退出本应用中的 ChatGPT 账号吗？")) return;
    setBusy(true);
    ++modelRequest.current;
    try {
      await gateway.logoutChatGpt();
      onModels(provider.providerId, undefined);
      await refreshStatus();
      onChanged(await gateway.getLlmConfiguration());
      notify.success("已退出 ChatGPT；其他应用的登录状态不受影响。");
    } catch (reason: unknown) {
      notify.error(aiAssistanceErrorMessage(reason));
    } finally {
      setBusy(false);
    }
  }

  async function loadModels(): Promise<void> {
    const revision = ++modelRequest.current;
    setBusy(true);
    try {
      const result = await gateway.fetchLlmProviderModels(provider.providerId, true);
      if (revision !== modelRequest.current) return;
      onModels(result.providerId, result.models);
      announceLlmConfigurationChanged();
      if (result.catalog?.stale) notify.warning(result.catalog.source === "bundled"
        ? "暂时无法刷新，正在使用内置模型目录；模型可用性可能已变化。"
        : "暂时无法刷新，正在使用该账号上次成功获取的模型目录。");
      else notify.success(`已刷新 ${result.models.length} 个 ChatGPT 模型。`);
    } catch (reason: unknown) {
      notify.error(aiAssistanceErrorMessage(reason));
    } finally {
      setBusy(false);
    }
  }

  return <section className="api-provider-panel">
    <header className="api-provider-panel-heading">
      <div><div className="api-provider-name"><h3>{provider.displayName}</h3><span>{status === undefined ? "检查中" : status.configured ? "已登录" : "未登录"}</span></div>
        <small>OpenAI 官方 Codex app-server{versions && ` ${versions.runtimeVersion}`}（仅用于登录与账号状态）· 与端口无关</small></div>
    </header>
    <SettingGroup label="ChatGPT 账号">
      <SettingRow title={status?.email ?? "ChatGPT 账号"} description="凭据保存在本应用独立的数据目录，无需安装 Codex CLI。"><span>{status === undefined ? "检查中" : status.configured ? "已登录" : "未登录"}</span></SettingRow>
      <CodexCatalogVersionSetting enabled={desktopAvailable} gateway={versionGateway} onChanged={setVersions}/>
      <SettingRow title="模型目录"><span>{models === undefined ? "尚未缓存" : models.length + " 个模型"}</span></SettingRow>
    </SettingGroup>
    {status === undefined
      ? <p className="api-settings-banner" role="status">正在检查内置运行时与账号状态…</p>
      : !status.runtimeAvailable && <p className="api-settings-banner">内置 Codex 运行时不可用，请重新安装或更新应用。</p>}
    <footer className="api-provider-actions">
      {loginId !== undefined
        ? <button disabled={!desktopAvailable} onClick={() => void cancelLogin()} type="button">取消登录</button>
        : status?.configured
        ? <button className="danger-primary" disabled={busy || !desktopAvailable} onClick={() => void logout()} type="button">退出登录</button>
        : <button disabled={busy || !desktopAvailable || status === undefined || status.runtimeAvailable === false} onClick={() => void login()} type="button">{busy ? "等待登录…" : "登录 OpenAI / ChatGPT"}</button>}
      <button disabled={busy || !desktopAvailable || !status?.configured} onClick={() => void loadModels()} type="button"><RefreshCw aria-hidden="true" size={14} />刷新模型列表</button>
    </footer>
  </section>;
}

export interface AiProviderSettingsProps {
  readonly versionGateway?: CodexCatalogVersionGateway;
  readonly desktopAvailable?: boolean;
  readonly gateway?: AiAssistanceGateway;
  readonly onClose: () => void;
}

function providerDraft(provider: LlmProviderConfig): LlmProviderConfigDraft {
  return {
    providerId: provider.providerId,
    enabled: provider.enabled,
    baseUrl: provider.baseUrl,
    timeoutMs: provider.timeoutMs,
  };
}

function llmProviderReady(provider: LlmProviderConfig): boolean {
  return provider.enabled && (provider.authMode !== "api-key" || provider.credentialConfigured);
}

function llmProviderOptionLabel(provider: LlmProviderConfig): string {
  if (!provider.enabled) return `${provider.displayName}（未启用）`;
  if (provider.authMode === "api-key" && !provider.credentialConfigured) return `${provider.displayName}（缺少 API key）`;
  return provider.displayName;
}

function ProviderEditor({
  desktopAvailable,
  gateway,
  models,
  onChanged,
  onModels,
  provider,
}: ProviderEditorProps): React.JSX.Element {
  const apiProviderId = provider.providerId as "deepseek" | "qwen";
  const [draft, setDraft] = useState<LlmProviderConfigDraft>(() => providerDraft(provider));
  const [apiKey, setApiKey] = useState("");
  const [keyEditing, setKeyEditing] = useState(!provider.credentialConfigured);
  const [keyVisible, setKeyVisible] = useState(false);
  const [action, setAction] = useState<ProviderAction>("idle");

  useEffect(() => {
    setDraft(providerDraft(provider));
    setApiKey("");
    setKeyEditing(!provider.credentialConfigured);
    setKeyVisible(false);
  }, [provider]);

  async function save(event: React.FormEvent<HTMLFormElement>): Promise<void> {
    event.preventDefault();
    const nextKey = apiKey.trim();
    if (draft.enabled && !provider.credentialConfigured && nextKey === "") {
      notify.error("启用该 Provider 前需要填写 API key。");
      return;
    }
    setAction("saving");
    let credentialUpdated = false;
    try {
      if (nextKey !== "") {
        await gateway.setCredential({ providerId: apiProviderId, apiKey: nextKey });
        credentialUpdated = true;
      }
      const configuration = await gateway.saveLlmProviderConfig({
        providerId: draft.providerId,
        enabled: draft.enabled,
        baseUrl: draft.baseUrl.trim() === provider.defaultBaseUrls[0] ? "" : draft.baseUrl,
        timeoutMs: draft.timeoutMs,
      });
      if (credentialUpdated || draft.baseUrl !== provider.baseUrl) onModels(provider.providerId, undefined);
      onChanged(configuration);
      notify.success(credentialUpdated ? "API key 与 Provider 设置已保存。" : "Provider 设置已保存。");
    } catch (reason: unknown) {
      const message = aiAssistanceErrorMessage(reason);
      if (credentialUpdated) {
        await gateway.getLlmConfiguration().then(onChanged).catch(() => undefined);
        notify.error(`API key 已安全保存，但 Provider 参数未保存：${message}`);
      } else {
        notify.error(message);
      }
    } finally {
      setAction("idle");
    }
  }

  async function loadModels(): Promise<void> {
    setAction("loading-models");
    try {
      const result = await gateway.fetchLlmProviderModels(provider.providerId);
      onModels(result.providerId, result.models);
      if (result.models.length === 0) {
        notify.warning("Provider 返回了空模型列表。");
      } else {
        notify.success(`已刷新 ${result.models.length} 个模型。`);
      }
    } catch (reason: unknown) {
      notify.error(aiAssistanceErrorMessage(reason));
    } finally {
      setAction("idle");
    }
  }

  async function deleteCredential(): Promise<void> {
    if (!window.confirm(`确定撤销 ${provider.displayName} 的本地 API key 吗？`)) return;
    setAction("deleting");
    try {
      await gateway.deleteCredential(apiProviderId);
      onModels(provider.providerId, undefined);
      const configuration = await gateway.getLlmConfiguration();
      onChanged(configuration);
      notify.success("本地 API key 已撤销；Provider 参数仍保留。");
    } catch (reason: unknown) {
      notify.error(aiAssistanceErrorMessage(reason));
    } finally {
      setAction("idle");
    }
  }

  const isBusy = action !== "idle";
  const availability = provider.credentialConfigured
    ? provider.enabled ? "已启用" : "已保存 · 未启用"
    : "未配置";

  return (
    <form className="api-provider-panel" onSubmit={(event) => void save(event)}>
      <header className="api-provider-panel-heading">
        <div>
          <div className="api-provider-name"><h3>{provider.displayName}</h3><span>{availability}</span></div>
          <small>OpenAI 兼容接口 · {provider.modelKinds.map((kind) => kind === "text" ? "文本" : kind === "embedding" ? "Embedding" : "视觉").join(" / ")}</small>
        </div>
        <label className="api-provider-toggle">
          <input className="settings-switch" role="switch" checked={draft.enabled} disabled={isBusy} onChange={(event) => setDraft({ ...draft, enabled: event.target.checked })} type="checkbox" />
          启用
        </label>
      </header>

      <div className="api-provider-fields setting-group">
        <SettingRow title="Base URL" description={provider.usesDefaultBaseUrl ? "使用默认地址；清空保存可恢复默认值。" : "自定义地址"}><input aria-label="Base URL" disabled={isBusy} onChange={event => setDraft({ ...draft, baseUrl: event.target.value })} value={draft.baseUrl} /></SettingRow>
        <SettingRow title="请求超时" description="单位：秒"><input aria-label="超时" disabled={isBusy} max={300} min={1} onChange={event => setDraft({ ...draft, timeoutMs: event.target.valueAsNumber * 1_000 })} type="number" value={draft.timeoutMs / 1_000} /></SettingRow>
        <SettingRow title="模型目录"><span>{models === undefined ? "尚未缓存" : models.length + " 个模型"}</span></SettingRow>
      </div>

      <div className="api-key-field setting-group">
        <div className="api-key-label"><span>API key</span><small>只写；保存后仅显示末四位</small></div>
        <div className="api-key-control">
          <div className="api-key-input-wrap">
            <input
              autoComplete="off"
              disabled={isBusy}
              onChange={(event) => setApiKey(event.target.value)}
              placeholder={keyEditing ? "输入新的 API key" : "未配置"}
              readOnly={!keyEditing}
              type={keyEditing && !keyVisible ? "password" : "text"}
              value={keyEditing ? apiKey : provider.credentialMask ?? ""}
            />
            {keyEditing && <ControlHint content={keyVisible ? "隐藏 API key" : "显示 API key"}><button
              aria-label={keyVisible ? "隐藏 API key" : "显示 API key"}
              aria-pressed={keyVisible}
              className="api-key-visibility"
              disabled={isBusy || apiKey.length === 0}
              onClick={() => setKeyVisible((current) => !current)}
              type="button"
            >
              {keyVisible ? <EyeOff aria-hidden="true" size={15} /> : <Eye aria-hidden="true" size={15} />}
            </button></ControlHint>}
          </div>
          {keyEditing ? (
            provider.credentialConfigured && <button disabled={isBusy} onClick={() => { setApiKey(""); setKeyEditing(false); setKeyVisible(false); }} type="button">取消</button>
          ) : (
            <button disabled={isBusy} onClick={() => { setKeyEditing(true); setKeyVisible(false); }} type="button"><RotateCcw aria-hidden="true" size={14} />更换</button>
          )}
        </div>
      </div>

      <footer className="api-provider-actions">
        <button disabled={isBusy || !desktopAvailable} type="submit"><Save aria-hidden="true" size={14} />{action === "saving" ? "正在保存…" : "保存 Provider"}</button>
        <button disabled={isBusy || !desktopAvailable || !provider.credentialConfigured} onClick={() => void loadModels()} type="button"><RefreshCw aria-hidden="true" size={14} />{action === "loading-models" ? "刷新中…" : "刷新模型列表"}</button>
        <button className="danger" disabled={isBusy || !desktopAvailable || !provider.credentialConfigured} onClick={() => void deleteCredential()} type="button">{action === "deleting" ? "正在撤销…" : "撤销 key"}</button>
      </footer>
    </form>
  );
}

function RouteRow({
  models,
  onChange,
  providers,
  route,
}: {
  readonly models: ModelsByProvider;
  readonly onChange: (selection: LlmModelSelection | undefined) => void;
  readonly providers: readonly LlmProviderConfig[];
  readonly route: LlmModelRoute;
}): React.JSX.Element {
  const eligible = providers.filter((provider) => provider.modelKinds.includes(route.modelKind));
  const providerId = route.selection?.providerId ?? "";
  const selectedProvider = eligible.find((provider) => provider.providerId === providerId);
  const selectedProviderReady = selectedProvider === undefined || llmProviderReady(selectedProvider);
  const modelId = route.selection?.modelId ?? "";
  const modelListId = `models-${route.routeKind}-${route.routeId}`;

  function selectProvider(value: string): void {
    if (value === "") {
      onChange(undefined);
      return;
    }
    onChange({ providerId: value as LlmProviderId, modelId });
  }

  return (
    <div className={`api-route-row${providerId === "openai" ? " api-route-row-reasoning" : ""}`}>
      <div><strong>{route.label}</strong><small>{route.routeKind === "use_case" ? `默认继承“${route.modelKind === "text" ? "文本模型" : route.modelKind}”` : route.modelKind}</small></div>
      <select aria-label={`${route.label} Provider`} onChange={(event) => selectProvider(event.target.value)} value={providerId}>
        <option value="">{route.routeKind === "use_case" ? "继承默认" : "未配置"}</option>
        {eligible.map((provider) => <option disabled={!llmProviderReady(provider)} key={provider.providerId} value={provider.providerId}>{llmProviderOptionLabel(provider)}</option>)}
      </select>
      <input
        aria-label={`${route.label} 模型`}
        disabled={providerId === "" || !selectedProviderReady}
        list={modelListId}
        onChange={(event) => onChange(providerId === "" ? undefined : { providerId: providerId as LlmProviderId, modelId: event.target.value })}
        placeholder={providerId === "" ? "先选择 Provider" : !selectedProviderReady ? "先启用 Provider" : models[providerId as LlmProviderId] === undefined ? "输入模型 ID，或先刷新模型列表" : "选择或输入模型 ID"}
        value={modelId}
      />
      <datalist id={modelListId}>{(models[providerId as LlmProviderId] ?? []).map((model) => <option key={model.id} value={model.id} />)}</datalist>
      {providerId === "openai" && <select
        aria-label={`${route.label} 推理强度`}
        disabled={modelId === ""}
        onChange={(event) => onChange({
          providerId: "openai",
          modelId,
          ...(event.target.value === "" ? {} : { reasoningEffort: event.target.value as NonNullable<LlmModelSelection["reasoningEffort"]> }),
        })}
        value={route.selection?.reasoningEffort ?? ""}
      >
        <option value="">模型默认</option>
        {(models.openai?.find((model) => model.id === modelId)?.supportedReasoningEfforts
          ?? ["low", "medium", "high", "xhigh"]).map((effort) => <option key={effort} value={effort}>{effort}</option>)}
      </select>}
    </div>
  );
}

function ModelRoutingEditor({
  configuration,
  gateway,
  models,
  onChanged,
}: {
  readonly configuration: LlmConfiguration;
  readonly gateway: AiAssistanceGateway;
  readonly models: ModelsByProvider;
  readonly onChanged: (configuration: LlmConfiguration) => void;
}): React.JSX.Element {
  const [routes, setRoutes] = useState<readonly LlmModelRoute[]>([...configuration.defaultModels, ...configuration.useCases]);
  const [saving, setSaving] = useState(false);

  useEffect(() => setRoutes([...configuration.defaultModels, ...configuration.useCases]), [configuration]);

  function updateRoute(target: LlmModelRoute, selection: LlmModelSelection | undefined): void {
    setRoutes((current) => current.map((route) => {
      if (route.routeKind !== target.routeKind || route.routeId !== target.routeId) return route;
      const { selection: _previous, ...base } = route;
      return selection === undefined ? base : { ...base, selection };
    }));
  }

  async function save(): Promise<void> {
    setSaving(true);
    try {
      const invalidRoute = routes.find((route) => {
        if (route.selection === undefined) return false;
        const provider = configuration.providers.find((item) => item.providerId === route.selection?.providerId);
        return provider === undefined || !llmProviderReady(provider);
      });
      if (invalidRoute !== undefined) {
        const provider = configuration.providers.find((item) => item.providerId === invalidRoute.selection?.providerId);
        notify.error(`请先启用 ${provider?.displayName ?? "所选 Provider"} 并确认凭据可用，再保存“${invalidRoute.label}”。`);
        return;
      }
      const drafts: LlmModelRouteDraft[] = routes.map((route) => ({
        routeKind: route.routeKind,
        routeId: route.routeId,
        ...(route.selection === undefined ? {} : { selection: route.selection }),
      }));
      const next = await gateway.saveLlmModelRoutes(drafts);
      onChanged(next);
      notify.success("默认模型与场景覆盖已保存。");
    } catch (reason: unknown) {
      notify.error(aiAssistanceErrorMessage(reason));
    } finally {
      setSaving(false);
    }
  }

  const defaultRoutes = routes.filter((route) => route.routeKind === "default");
  const useCaseRoutes = routes.filter((route) => route.routeKind === "use_case");
  return (
    <section className="api-routing-panel">
      <header><div><h3>模型分配</h3><p>先设置各类默认模型；只有确有需要时，再为具体功能覆盖。</p></div><button disabled={saving} onClick={() => void save()} type="button"><Save aria-hidden="true" size={14} />{saving ? "保存中…" : "保存分配"}</button></header>
      <div className="api-route-group"><h4>默认模型</h4>{defaultRoutes.map((route) => <RouteRow key={route.routeId} models={models} onChange={(selection) => updateRoute(route, selection)} providers={configuration.providers} route={route} />)}</div>
      <div className="api-route-group"><h4>按功能覆盖</h4>{useCaseRoutes.map((route) => <RouteRow key={route.routeId} models={models} onChange={(selection) => updateRoute(route, selection)} providers={configuration.providers} route={route} />)}</div>
    </section>
  );
}

export function AiProviderSettings({
  versionGateway,
  desktopAvailable = hasPlatformCapabilities(),
  gateway: providedGateway,
  onClose,
}: AiProviderSettingsProps): React.JSX.Element {
  const defaultGateway = useMemo(() => getAiAssistanceGateway(), []);
  const gateway = providedGateway ?? defaultGateway;
  const [configuration, setConfiguration] = useState<LlmConfiguration>();
  const [models, setModels] = useState<ModelsByProvider>(() => readAllCachedLlmModels());
  const [loading, setLoading] = useState(true);
  const [loadFailed, setLoadFailed] = useState(false);
  const acceptConfiguration = useCallback((next: LlmConfiguration): void => {
    setConfiguration(next);
    announceLlmConfigurationChanged();
  }, []);
  const acceptModels = useCallback((providerId: LlmProviderId, nextModels: readonly LlmModelDescriptor[] | undefined): void => {
    if (nextModels === undefined) removeCachedLlmModels(providerId);
    else cacheLlmModels(providerId, nextModels);
    setModels((current) => {
      if (nextModels !== undefined) return { ...current, [providerId]: nextModels };
      const { [providerId]: _removed, ...rest } = current;
      return rest;
    });
  }, []);

  const refresh = useCallback(async (): Promise<void> => {
    setLoading(true);
    setLoadFailed(false);
    try {
      setConfiguration(await gateway.getLlmConfiguration());
    } catch (reason: unknown) {
      setLoadFailed(true);
      notify.error(aiAssistanceErrorMessage(reason));
    } finally {
      setLoading(false);
    }
  }, [gateway]);

  useEffect(() => {
    let current = true;
    void gateway.getLlmConfiguration()
      .then((next) => { if (current) setConfiguration(next); })
      .catch((reason: unknown) => {
        if (!current) return;
        setLoadFailed(true);
        notify.error(aiAssistanceErrorMessage(reason));
      })
      .finally(() => { if (current) setLoading(false); });
    return () => { current = false; };
  }, [gateway]);

  return (
    <Dialog open portal backdrop={false} hideOnInteractOutside={false} onClose={onClose} aria-labelledby="api-settings-title"
      className="api-settings-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <section className="api-settings-dialog settings-api-dialog">
        <header className="api-settings-heading">
          <div><span className="api-settings-icon" aria-hidden="true"><KeyRound size={17} strokeWidth={1.8} /></span><div><div className="api-title-row"><h2 id="api-settings-title">LLM API 设置</h2><SettingsHint label="API 配置与隐私说明">API key 仅保存在本机凭据文件；Base URL、默认模型与功能覆盖保存在本地数据库。模型目录首次读取后缓存在本机；“刷新模型列表”只请求 Provider 的 <code>/models</code>，不会发送论文或笔记内容。</SettingsHint></div><p>Provider、模型目录与功能路由</p></div></div>
          <button aria-label="关闭 API 设置" autoFocus onClick={onClose} type="button"><X aria-hidden="true" size={17} /></button>
        </header>

        <div className="api-settings-body">
          {!desktopAvailable && <div className="api-settings-banner">浏览器预览不会保存 API key，也不会调用真实 Provider。请在桌面应用中配置。</div>}
          {loading && configuration === undefined && <div className="api-settings-empty" role="status">正在读取 LLM 配置…</div>}
          {loadFailed && configuration === undefined && <div className="api-settings-empty">配置尚未载入。<button onClick={() => void refresh()} type="button">重试</button></div>}
          {configuration?.providers.map((provider) => {
            const props = { desktopAvailable, gateway, ...(models[provider.providerId] === undefined ? {} : { models: models[provider.providerId] }), onChanged: acceptConfiguration, onModels: acceptModels, provider };
            return provider.authMode === "chatgpt-login"
              ? <ChatGptEditor key={provider.providerId} {...props} versionGateway={versionGateway} />
              : <ProviderEditor key={provider.providerId} {...props} />;
          })}
          {configuration !== undefined && <ModelRoutingEditor configuration={configuration} gateway={gateway} models={models} onChanged={acceptConfiguration} />}
        </div>
      </section>
    </Dialog>
  );
}
