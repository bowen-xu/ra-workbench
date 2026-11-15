import { useState } from "react";
import { useAppRuntime } from "../../runtime/AppRuntimeProvider";
import { notify } from "../../services/notifications";
import { SettingGroup, SettingRow } from "./SettingsLayout";
import { SettingsSelect } from "./SettingsSelect";
import { flushStorage } from "../../runtime/storagePersistence";

/** This setting belongs to App; previews only supply its runtime capability. */
export function PreviewDataSettings(): React.JSX.Element | null {
  const runtime = useAppRuntime(), controls = runtime.preview;
  const [delayMs, setDelayMs] = useState(controls?.delayMs ?? 100);
  const [failure, setFailure] = useState(controls?.nextFailure ?? "none");
  if (!controls) return null;
  return <section className="settings-section" aria-label="预览数据">
    <h3>预览数据</h3>
    <p>当前全部数据为合成数据，保存在独立的 IndexedDB 预览空间。刷新保留已保存的编辑结果；不调用真实模型、桌面数据库或系统服务。</p>
    <SettingGroup label="预览数据"><SettingRow title="模拟延迟" description="每次操作的等待时间，单位为毫秒。"><input aria-label="模拟延迟（毫秒）" type="number" min={0} max={10000} value={delayMs} onChange={event => setDelayMs(Number(event.target.value))} /></SettingRow>
    <SettingRow title="下一次操作的失败场景"><SettingsSelect aria-label="下一次操作的失败场景" value={failure} onChange={event => setFailure(event.target.value)} options={[
      { value: "none", label: "无" }, { value: "save", label: "文件或数据保存失败" }, { value: "compile", label: "编译失败" }, { value: "agent", label: "Agent 提交失败" }, { value: "docling", label: "文档解析失败" }, { value: "platform", label: "平台操作失败" },
    ]} /></SettingRow>
    <SettingRow title="模拟设置" description="失败场景只作用于下一次对应操作。"><button type="button" onClick={() => { void (async () => {
      controls.configure({ delayMs, nextFailure: failure === "none" ? null : failure }); await flushStorage(runtime.storage);
      notify.success("模拟设置已保存，失败只作用于下一次对应操作。");
    })().catch(reason => notify.error(reason instanceof Error ? reason.message : String(reason))); }}>保存模拟设置</button></SettingRow>
    <SettingRow title="重置预览数据" description="清除预览中的编辑，恢复初始合成数据。"><button type="button" onClick={() => { void Promise.resolve().then(() => controls.reset()).catch(reason => notify.error(reason instanceof Error ? reason.message : String(reason))); }}>重置预览数据</button></SettingRow>
    </SettingGroup>
  </section>;
}
