import {useEffect, useState} from "react";
import {Select, SelectItem, SelectProvider} from "@ariakit/react";
import {Check, ChevronDown, RefreshCw} from "lucide-react";
import {ControlHint} from "../../components/ControlHint";
import {SelectPopover} from "../../components/overlay/OverlayPopup";
import {codexCatalogVersionGateway, type CodexCatalogVersions, type CodexCatalogVersionGateway} from "../../services/codexCatalogVersionGateway";
import {aiAssistanceErrorMessage} from "../../services/aiAssistanceGateway";
import {notify} from "../../services/notifications";
import {SettingRow} from "./SettingsLayout";
import {announceLlmConfigurationChanged} from "../models/llmConfigurationEvents";
import "./CodexCatalogVersionSetting.css";

export function CodexCatalogVersionSetting({enabled, gateway = codexCatalogVersionGateway, onChanged}: {
  readonly enabled: boolean;
  readonly gateway?: CodexCatalogVersionGateway | undefined;
  readonly onChanged: (value: CodexCatalogVersions) => void;
}): React.JSX.Element {
  const [value, setValue] = useState<CodexCatalogVersions>();
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!enabled) return;
    let active = true;
    void gateway.getVersions().then(next => {if (active) {setValue(next); onChanged(next);}})
      .catch(reason => {if (active) {setFailed(true); notify.error(aiAssistanceErrorMessage(reason));}});
    return () => {active = false;};
  }, [enabled, gateway, onChanged]);
  async function update(version?: string): Promise<void> {
    setBusy(true);
    try {
      const next = version === undefined ? await gateway.getVersions(true) : await gateway.selectVersion(version);
      setValue(next); setFailed(false); onChanged(next);
      if (version !== undefined) announceLlmConfigurationChanged();
      if (version === undefined) notify.success("已刷新可用 Codex CLI 版本；模型列表获取版本保持不变。");
    } catch (reason) {notify.error(aiAssistanceErrorMessage(reason));}
    finally {setBusy(false);}
  }
  return <SettingRow title="Codex CLI 版本号" description="仅供获取模型列表使用，不表示更新版本或使用的实际版本">
    <div className="codex-catalog-version-control" aria-busy={busy}>
      <SelectProvider value={value?.selectedVersion ?? ""} placement="bottom-end" setValue={next => {if (typeof next === "string" && next && next !== value?.selectedVersion) void update(next);}}>
        <Select className="codex-catalog-version-select" aria-label="Codex CLI 版本号" disabled={!enabled || busy || !value}>
          <span>{value?.selectedVersion ?? (enabled ? failed ? "读取失败" : "读取中…" : "仅桌面可用")}</span><ChevronDown size={16} aria-hidden="true"/>
        </Select>
        <SelectPopover className="codex-catalog-version-options" aria-label="可用 Codex CLI 版本" gutter={4}>
          {value?.availableVersions.map(version => <SelectItem className="codex-catalog-version-option" key={version} value={version}>
            <span>{version}</span>{version === value.selectedVersion && <Check size={19} aria-hidden="true"/>}
          </SelectItem>)}
        </SelectPopover>
      </SelectProvider>
      <ControlHint content="获取最新 Codex CLI 版本号"><button className="codex-catalog-version-refresh" type="button" aria-label="刷新 Codex CLI 版本号" disabled={!enabled || busy || (!value && !failed)} onClick={() => void update()}><RefreshCw className={busy ? "codex-version-refreshing" : undefined} size={15} aria-hidden="true"/></button></ControlHint>
    </div>
  </SettingRow>;
}
