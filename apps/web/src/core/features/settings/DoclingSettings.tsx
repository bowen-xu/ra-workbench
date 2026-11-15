import { doclingErrorMessage } from "../../services/doclingError";
import { SettingGroup, SettingRow, SettingsHint } from "./SettingsLayout";
import { hasPlatformCapabilities } from "../../runtime/appRuntime";
import { useCallback, useEffect, useMemo, useState } from "react";

import {
  getDoclingGateway,
  type DoclingServiceStatus,
  type DoclingModelStatus,
} from "../../services/doclingGateway";
import { Plug, Unplug, LoaderCircle, HardDrive } from "lucide-react";
import { ControlHint } from "../../components/ControlHint";
import { DoclingModelAction } from "./DoclingModelAction";
import { useDoclingSettings } from "./useDoclingSettings";
import { notify } from "../../services/notifications";

function statusLabel(status: DoclingServiceStatus | undefined): string {
  if (status === undefined) return "未知";
  if (status.state === "connected") {
    const endpoint = status.baseUrl == null ? "" : ` · ${status.baseUrl}`;
    const clients =
      status.activeClientCount == null
        ? ""
        : ` · ${status.activeClientCount} 个客户端`;
    const requests =
      status.activeRequestCount == null
        ? ""
        : ` · ${status.activeRequestCount} 个任务`;
    return `已连接${endpoint}${clients}${requests}`;
  }
  if (status.state === "discovering") return "正在查找已有服务";
  if (status.state === "starting") return "启动中";
  if (status.state === "reconnecting") return "正在重新连接";
  if (status.state === "incompatible") return "服务协议不兼容";
  if (status.state === "unresponsive") return "服务无响应";
  if (status.state === "models_unavailable") return "尚未安装可用模型";
  if (status.state === "browser_unavailable") return "浏览器预览不可用";
  return "本客户端未连接";
}

export function DoclingSettings({ onManageModels, active: visible = true }: { readonly onManageModels?: () => void; readonly active?: boolean }): React.JSX.Element {
  const gateway = useMemo(() => getDoclingGateway(), []);
  const { draft, loaded, update, flush } = useDoclingSettings(gateway, visible);
  const [status, setStatus] = useState<DoclingServiceStatus>();
  const [busy, setBusy] = useState<"load" | "connect" | "disconnect" | "ping">(
    "load",
  );
  const [lastPingMs, setLastPingMs] = useState<number>();
  const [models, setModels] = useState<DoclingModelStatus>();

  const refreshStatus = useCallback(async (): Promise<void> => {
    const [service, resources] = await Promise.all([
      gateway.getStatus(),
      gateway.getModelStatus(),
    ]);
    setStatus(service);
    setModels(resources);
  }, [gateway]);

  useEffect(() => {
    if (!visible) return;
    let active = true;
    void gateway
      .getStatus()
      .then((nextStatus) => {
        if (active) setStatus(nextStatus);
      })
      .catch((reason: unknown) => notify.error(doclingErrorMessage(reason)))
      .finally(() => {
        if (active) setBusy("load");
      });
    return () => {
      active = false;
    };
  }, [gateway, visible]);

  useEffect(() => {
    if (!visible) return;
    const interval = window.setInterval(() => {
      if (hasPlatformCapabilities())
        void refreshStatus().catch(() => undefined);
    }, 5_000);
    return () => window.clearInterval(interval);
  }, [refreshStatus, visible]);

  useEffect(() => {
    if (!visible) return;
    let active = true;
    void gateway
      .getModelStatus()
      .then((value) => {
        if (active) setModels(value);
      })
      .catch((reason) => notify.error(doclingErrorMessage(reason)));
    return () => {
      active = false;
    };
  }, [gateway, visible]);

  async function connect(): Promise<void> {
    setBusy("connect");
    try {
      await flush();
      setStatus(await gateway.connectService());
      notify.success("已连接 Docling 服务");
    } catch (reason: unknown) {
      notify.error(doclingErrorMessage(reason));
      await refreshStatus().catch(() => undefined);
    } finally {
      setBusy("load");
    }
  }

  async function disconnect(): Promise<void> {
    setBusy("disconnect");
    try {
      setStatus(await gateway.disconnectService());
      notify.success("本客户端已断开；服务将按 heartbeat 和空闲时间自行退出");
    } catch (reason: unknown) {
      notify.error(doclingErrorMessage(reason));
    } finally {
      setBusy("load");
    }
  }

  async function ping(): Promise<void> {
    setBusy("ping");
    try {
      const result = await gateway.pingService();
      setStatus(result.status);
      setLastPingMs(result.latencyMs);
      notify.success(`Docling Ping 成功 · ${result.latencyMs} ms`);
    } catch (reason: unknown) {
      notify.error(doclingErrorMessage(reason));
      await refreshStatus().catch(() => undefined);
    } finally {
      setBusy("load");
    }
  }

  const disabled = !loaded || busy !== "load" || !hasPlatformCapabilities();
  const connected = status?.state === "connected";
  const connectionLabel =
    busy === "connect"
      ? "正在连接服务"
      : busy === "disconnect"
        ? "正在断开本客户端"
        : connected
          ? "断开本客户端"
          : "连接服务";
  const ConnectionIcon =
    busy === "connect" || busy === "disconnect"
      ? LoaderCircle
      : connected
        ? Unplug
        : Plug;

  return (
    <section className="settings-card docling-settings-card">
      <h2 className="settings-title-with-hint">
        Docling 结构化解析{" "}
        <SettingsHint label="Docling 服务说明">
          服务只绑定 127.0.0.1。HTTP 承载转换，WebSocket heartbeat
          维持本客户端租约；断开不会终止其他客户端或直接杀死 daemon。
        </SettingsHint>
      </h2>
      <p>使用原生 Rust Docling 与 FP32 模型解析 PDF，无需 Python 或 Conda。</p>
      <SettingGroup label="Docling 服务配置">
        <SettingRow title="启用本地 Docling 服务">
          <input
            aria-label="启用本地 Docling 服务"
            className="settings-switch"
            role="switch"
            type="checkbox"
            checked={draft.enabled}
            disabled={disabled}
            onChange={(event) => update({ enabled: event.target.checked })}
          />
        </SettingRow>
        <SettingRow title="模型更新源">
          <input
            aria-label="模型更新源"
            disabled={disabled}
            value={draft.modelManifestUrl}
            onChange={(event) =>
              update({ modelManifestUrl: event.target.value })
            }
            onBlur={() => void flush().catch(() => undefined)}
            placeholder="http://127.0.0.1:4174/models/docling/latest.json"
          />
        </SettingRow>
        {(
          [
            ["defaultPort", "默认端口", 1, 65535],
            ["fallbackStartPort", "fallback 起始", 1, 65535],
            ["fallbackEndPort", "fallback 结束", 1, 65535],
            ["idleShutdownMs", "空闲退出（毫秒）", 60_000, undefined],
            ["requestTimeoutMs", "解析无响应超时（毫秒）", 10_000, undefined],
          ] as const
        ).map(([field, label, min, max]) => (
          <SettingRow key={field} title={label}>
            <input
              aria-label={label}
              disabled={disabled}
              type="number"
              min={min}
              max={max}
              value={draft[field]}
              onChange={(event) =>
                update({ [field]: Number(event.target.value) })
              }
              onBlur={() => void flush().catch(() => undefined)}
            />
          </SettingRow>
        ))}
      </SettingGroup>
      <div className="settings-panel-gap">
        <SettingGroup label="Docling FP32 模型">
          <SettingRow
            title="模型资源"
            description={
              models?.installedVersion
                ? `当前版本：${models.installedVersion}${models.availableVersion ? ` · 更新源版本：${models.availableVersion}` : ""}`
                : "尚未安装。模型独立于软件安装包，修改更新源后检查并下载。"
            }
          >
            <ControlHint content="管理模型存储"><button aria-label="管理模型存储" className="settings-icon-button" type="button" onClick={onManageModels} disabled={!onManageModels}><HardDrive aria-hidden="true" size={16} /></button></ControlHint>
            <DoclingModelAction
              gateway={gateway}
              models={models}
              setModels={setModels}
              disabled={disabled}
              flush={flush}
            />
          </SettingRow>
          {(models?.state === "downloading" ||
            models?.state === "verifying") && (
            <SettingRow
              title={models.state === "verifying" ? "校验模型" : "下载进度"}
              description={`${Math.round(models.downloadedBytes / 1024 / 1024)} / ${Math.round(models.totalBytes / 1024 / 1024)} MB`}
            >
              <progress
                aria-label="模型下载进度"
                max={models.totalBytes || 1}
                value={models.downloadedBytes}
              />
            </SettingRow>
          )}
          {models?.lastError && (
            <p role="status" className="storage-footnote">
              {models.lastError}
            </p>
          )}
        </SettingGroup>
      </div>
      <div className="settings-panel-gap">
        <SettingGroup label="Docling 连接">
          <SettingRow
            title="服务连接"
            description={
              !hasPlatformCapabilities()
                ? "浏览器预览不能启动本地 Docling 服务，请使用桌面模式。"
                : statusLabel(status)
            }
          >
            <ControlHint content={connectionLabel} wrap>
              <button
                className="settings-icon-button"
                aria-label={connectionLabel}
                disabled={disabled || (!connected && !draft.enabled)}
                onClick={() => void (connected ? disconnect() : connect())}
                type="button"
              >
                <ConnectionIcon
                  size={16}
                  aria-hidden="true"
                  className={
                    busy === "connect" || busy === "disconnect"
                      ? "settings-action-spinning"
                      : undefined
                  }
                />
              </button>
            </ControlHint>
          </SettingRow>
          <SettingRow
            title="连接测试"
            description={
              lastPingMs === undefined
                ? "检查当前连接的响应情况。"
                : `最近 Ping：${lastPingMs} ms`
            }
          >
            <button
              disabled={disabled || status?.state !== "connected"}
              onClick={() => void ping()}
              type="button"
            >
              {busy === "ping" ? "Ping 中…" : "Ping"}
            </button>
          </SettingRow>
        </SettingGroup>
      </div>
      {status?.lastError !== undefined && (
        <p className="storage-footnote" role="status">
          {status.lastError}
        </p>
      )}
    </section>
  );
}
