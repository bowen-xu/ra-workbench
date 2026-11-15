import { appStorageEventKey, getAppStorage } from "../../runtime/appStorage";
import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { notify } from "../../services/notifications";
import { SettingGroup, SettingRow } from "./SettingsLayout";

import "./DeveloperMode.css";

const key = "research-assistant.developer-mode";
const Context = createContext({ enabled: false, setEnabled: (_value: boolean) => {} });
export function DeveloperModeProvider({ children }: { readonly children: ReactNode }): React.JSX.Element {
  const [enabled, setEnabled] = useState(() => {
    try { return getAppStorage().getItem(key) === "true"; } catch { return false; }
  });
  useEffect(() => {
    const sync = (event: StorageEvent) => {
      if (event.key === appStorageEventKey(key)) setEnabled(event.newValue === "true");
      else if (event.key === null) setEnabled(getAppStorage().getItem(key) === "true");
    };
    window.addEventListener("storage", sync);
    return () => window.removeEventListener("storage", sync);
  }, []);
  function update(value: boolean): void {
    setEnabled(value);
    try { getAppStorage().setItem(key, String(value)); }
    catch { notify.error("开发者模式已生效，但无法保存设置。"); }
  }
  return <Context.Provider value={{ enabled, setEnabled: update }}>{children}</Context.Provider>;
}
export const useDeveloperMode = () => useContext(Context);
export function DeveloperModeToggle(): React.JSX.Element {
  const { enabled, setEnabled } = useDeveloperMode();
  return <label className="developer-mode-toggle"><input type="checkbox" checked={enabled} onChange={event => setEnabled(event.target.checked)} />开发者模式</label>;
}
export function DeveloperModeSettings(): React.JSX.Element {
  const { enabled, setEnabled } = useDeveloperMode();
  return <section className="settings-section developer-setting"><h3>开发工具</h3>
    <SettingGroup label="开发工具"><SettingRow title="开发者模式" description="查看消息原始数据，并记录本次打开笔记后的调试操作；可在笔记的更多操作菜单复制、下载或清空。日志仅存内存，关闭笔记、关闭此模式或刷新后清空；中途开启从当时开始记录。">
      <input className="settings-switch" type="checkbox" role="switch" aria-label="开发者模式" checked={enabled} onChange={event => setEnabled(event.target.checked)} />
    </SettingRow></SettingGroup></section>;
}
