import { RefreshCw } from "lucide-react";
import { ControlHint } from "../../components/ControlHint";
import { SettingGroup, SettingRow, SettingsHint } from "./SettingsLayout";
import { useEffect, useMemo, useRef, useState } from "react";

import { isDesktopRuntime } from "../../runtime/platformCore";
import {
  getBrowserConnectorGateway,
  type BrowserConnectorGateway,
  type BrowserConnectorStatus,
} from "../../services/browserConnectorGateway";
import { notify } from "../../services/notifications";
import "./BrowserConnectorSettings.css";

interface BrowserConnectorSettingsProps {
  readonly gateway?: BrowserConnectorGateway;
  readonly active?: boolean;
}

export function BrowserConnectorSettings({ gateway: suppliedGateway, active = true }: BrowserConnectorSettingsProps = {}): React.JSX.Element {
  const gateway = useMemo(() => suppliedGateway ?? getBrowserConnectorGateway(), [suppliedGateway]);
  const [status, setStatus] = useState<BrowserConnectorStatus>();
  const [action, setAction] = useState<"idle" | "load">("load");
  const mounted = useRef(true);
  const isDesktop = isDesktopRuntime();
  const loaded = useRef<BrowserConnectorGateway | undefined>(undefined);

  useEffect(() => {
    mounted.current = true;
    if (!active || loaded.current === gateway) return () => {mounted.current = false;};
    let current = true;
    void gateway.getStatus().then(value => {
      if (current) {setStatus(value); loaded.current = gateway;}
    }).catch(() => {
      if (current) {
        setStatus({ endpoint: "", available: false, message: "Chrome 插件连接状态不可用，请刷新重试。" });
        notify.error("无法读取 Chrome 插件连接状态，请刷新重试。");
      }
    }).finally(() => { if (current) setAction("idle"); });
    return () => { current = false; mounted.current = false; };
  }, [gateway, active]);

  async function refresh(): Promise<void> {
    setAction("load");
    try {
      const value = await gateway.getStatus();
      if (mounted.current) setStatus(value);
    } catch {
      if (mounted.current) {
        setStatus({ endpoint: "", available: false, message: "Chrome 插件连接状态不可用，请刷新重试。" });
        notify.error("无法读取 Chrome 插件连接状态，请刷新重试。");
      }
    } finally { if (mounted.current) setAction("idle"); }
  }

  const disabled = action !== "idle" || !isDesktop;
  return <section className="settings-card browser-connector-settings">
    <h2 className="settings-title-with-hint">Chrome 文献采集 <SettingsHint label="Chrome 文献采集说明">无需填写配对码。插件核对应用、协议和运行环境后才发送文献；端口被其他软件占用时停止传输。</SettingsHint></h2>
    <p>自动检测浏览器插件，采集文献与可访问的 PDF。</p>
    <SettingGroup label="Chrome 文献采集连接">
      <SettingRow title="连接状态" description={!isDesktop ? "浏览器预览不能连接 Chrome 插件，请使用桌面应用。" : "自动检测，无需填写配对码。"}>
        <span className={status?.available ? "settings-connection-state connected" : "settings-connection-state"} role="status">{status?.message ?? (action === "load" ? "正在读取连接状态…" : "连接状态不可用")}</span>
      </SettingRow>
      {status?.endpoint && <SettingRow title="本机地址"><code>{status.endpoint}</code></SettingRow>}
      <SettingRow title="检测连接" description="打开应用后自动检测，也可手动刷新。"><ControlHint content={action === "load" ? "正在刷新连接状态" : "刷新连接状态"} wrap><button className="settings-icon-button" aria-label={action === "load" ? "正在刷新连接状态" : "刷新连接状态"} disabled={disabled} onClick={() => void refresh()} type="button"><RefreshCw size={16} aria-hidden="true" className={action === "load" ? "settings-action-spinning" : undefined} /></button></ControlHint></SettingRow>
    </SettingGroup>
  </section>;
}
