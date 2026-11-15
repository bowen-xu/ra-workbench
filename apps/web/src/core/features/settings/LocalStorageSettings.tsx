import { SettingGroup, SettingRow, SettingsHint } from "./SettingsLayout";
import { useNotificationState } from "../../services/useNotificationState";
import { hasPlatformCapabilities } from "../../runtime/appRuntime";
import { useCallback, useEffect, useMemo, useState } from "react";
import { isDesktopRuntime } from "../../runtime/platformCore";
import type { ContentGcRunReport, ContentStorageReport } from "@research-assistant/data";

import { getContentStorageGateway } from "../../services/contentStorageGateway";
import { getDoclingGateway, type DoclingModelStorage } from "../../services/doclingGateway";
import { doclingErrorMessage } from "../../services/doclingError";

function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1);
  return `${(value / 1024 ** index).toLocaleString("zh-CN", { maximumFractionDigits: index === 0 ? 0 : 1 })} ${units[index]}`;
}

function errorMessage(reason: unknown): string {
  if (typeof reason === "object" && reason !== null && "message" in reason) {
    return String((reason as { readonly message: unknown }).message);
  }
  return reason instanceof Error ? reason.message : String(reason);
}

export function LocalStorageSettings({ active = true }: { readonly active?: boolean }): React.JSX.Element {
  const gateway = useMemo(() => getContentStorageGateway(), []);
  const modelsGateway = useMemo(() => getDoclingGateway(), []);
  const [models, setModels] = useState<DoclingModelStorage>();
  const [report, setReport] = useState<ContentStorageReport>();
  const [action, setAction] = useState<"idle" | "scan" | "clean" | "models">("idle");
  const [includeGracePeriod, setIncludeGracePeriod] = useState(false);
  const [result, setResult] = useState<ContentGcRunReport>();
  const [, setNotice] = useNotificationState("info");
  const [, setError] = useNotificationState("error");

  const refresh = useCallback(async (): Promise<void> => {
    const [content, resources] = await Promise.all([gateway.getReport(), modelsGateway.getModelStorage()]);
    setReport(content); setModels(resources);
  }, [gateway, modelsGateway]);

  useEffect(() => {
    if (!active) return;
    void refresh().catch((reason: unknown) => setError(errorMessage(reason)));
  }, [active, refresh]);

  useEffect(() => {
    if (!active) return;
    const timer = window.setInterval(() => { void refresh().catch(reason => setError(errorMessage(reason))); }, 5000);
    return () => window.clearInterval(timer);
  }, [active, refresh]);

  async function deleteModels(): Promise<void> {
    if (!window.confirm("删除本机全部 Docling 模型并释放空间？\n\n将断开本客户端；其他客户端或解析任务使用中时不会删除。已保存的解析结果保留，再次解析前需重新下载模型。开发版和正式版共用这些模型。")) return;
    setAction("models");
    try {
      await modelsGateway.deleteModels();
      await refresh();
      setNotice("Docling 模型已删除，空间已释放；再次解析前请重新下载模型。");
    } catch (reason) { setError(doclingErrorMessage(reason)); await refresh().catch(() => undefined); }
    finally { setAction("idle"); }
  }

  async function scan(): Promise<void> {
    setAction("scan");
    setError(undefined);
    setNotice(undefined);
    try {
      const marked = await gateway.scan();
      await refresh();
      setNotice(`扫描完成，新增 ${marked.toLocaleString("zh-CN")} 个候选对象。`);
    } catch (reason: unknown) {
      setError(errorMessage(reason));
    } finally {
      setAction("idle");
    }
  }

  async function clean(): Promise<void> {
    if (includeGracePeriod && !window.confirm(
      "确定同时清理宽限期内的无引用对象吗？\n\n仍被正文、历史、文献或稿件引用的内容不会删除；无引用对象将立即永久释放。",
    )) return;
    setAction("clean");
    setError(undefined);
    setNotice(undefined);
    try {
      const nextResult = await gateway.clean(includeGracePeriod);
      setResult(nextResult);
      await refresh();
      setNotice(`清理完成，释放 ${formatBytes(nextResult.deletedBytes)}（${nextResult.deletedObjectCount.toLocaleString("zh-CN")} 个对象）。`);
    } catch (reason: unknown) {
      const message = errorMessage(reason);
      setError(message.includes("取消") ? "清理已取消，已完成的原子步骤保持一致。" : message);
      await refresh().catch(() => undefined);
    } finally {
      setAction("idle");
    }
  }

  async function cancel(): Promise<void> {
    if (await gateway.cancel()) setNotice("已请求取消；当前原子步骤完成后停止。");
  }

  const categories = report === undefined ? [] : [
    { label: "当前内容", bytes: report.currentReferencedBytes, color: "#658970" },
    { label: "历史版本", bytes: report.historyOnlyBytes, color: "#9aac88" },
    { label: "宽限期内", bytes: report.graceBytes, color: "#b8c8b7" },
    { label: "可回收", bytes: report.reclaimableBytes, color: "#b3b98c" },
  ];
  const categorized = categories.reduce((sum, item) => sum + item.bytes, 0);
  if (report && report.totalBytes > categorized) categories.push({ label: "其他对象", bytes: report.totalBytes - categorized, color: "#c9c9ba" });
  if (models) categories.push({ label: "Docling 模型", bytes: models.totalBytes, color: "#8a9fac" });
  const totalBytes = (report?.totalBytes ?? 0) + (models?.totalBytes ?? 0);
  const denominator = Math.max(totalBytes, categorized, 1);
  return <section className="settings-card local-storage-settings">
    <h2>本地存储</h2><p>管理保存在这台设备上的图片、附件、历史版本和模型资源。</p>
    {report?.isLowDiskSpace && <div className="storage-banner warning" role="alert">磁盘可用空间偏低。建议先扫描并清理到期对象。</div>}
    <section className="settings-storage-overview">
      <h3>当前设备</h3><p>{hasPlatformCapabilities() ? "内容保存在本机，按引用状态分类。" : "浏览器存储显示当前适配器的用量估算，自动回收只在桌面应用中运行。"}</p>
      <div className="settings-storage-total"><div className="settings-storage-summary">
        <strong>{report && models ? `${formatBytes(totalBytes)} 已使用` : "正在读取存储报告…"}</strong>
        {report && <span className="settings-storage-available">{isDesktopRuntime() ? "磁盘可用空间" : "站点可用配额"} {formatBytes(report.availableBytes)}</span>}
      </div><button type="button" disabled={action !== "idle"} onClick={() => void refresh().catch(reason => setError(errorMessage(reason)))}>刷新</button></div>
      <div className="settings-capacity-bar" role="img" aria-label={totalBytes ? "存储容量分布，分类数值见下方图例" : "尚无存储对象"}>{categories.filter(item => item.bytes > 0).map(item => <span key={item.label} style={{ width: `${item.bytes / denominator * 100}%`, background: item.color }} />)}</div>
      <div className="settings-storage-legend">{categories.map(item => <div key={item.label}><span><i style={{ background: item.color }} />{item.label}</span><strong>{formatBytes(item.bytes)}</strong></div>)}</div>
    </section>
    <SettingGroup label="模型资源">
      <SettingRow title="Docling FP32 模型" description={models ? `${formatBytes(models.totalBytes)}${models.installedVersion ? ` · ${models.installedVersion}` : " · 未安装可用模型"} · 开发版与正式版共用` : "正在读取模型占用…"}>
        <button type="button" disabled={action !== "idle" || !models?.totalBytes || models.downloading || !hasPlatformCapabilities()} onClick={() => void deleteModels()}>{action === "models" ? "正在删除…" : "删除模型"}</button>
      </SettingRow>
    </SettingGroup>
    <div className="settings-panel-gap"><SettingGroup label="存储清理">
      <SettingRow title="扫描可回收空间" description="检查已不再被正文或保留历史引用的对象。"><button disabled={action !== "idle" || !hasPlatformCapabilities()} onClick={() => void scan()} type="button">{action === "scan" ? "正在扫描…" : "扫描"}</button></SettingRow>
      <SettingRow title="清理到期对象" description={report ? `当前可回收 ${formatBytes(report.reclaimableBytes)} · ${report.reclaimableObjectCount.toLocaleString("zh-CN")} 个对象` : "读取报告后可清理到期对象。"}><button className="settings-primary" disabled={action !== "idle" || !hasPlatformCapabilities()} onClick={() => void clean()} type="button">{action === "clean" ? "正在清理…" : "清理"}</button></SettingRow>
      <SettingRow title="同时清理宽限期内的无引用对象" description="提前释放空间，执行时需再次确认。"><input aria-label="同时清理宽限期内的无引用对象（需再次确认）" className="settings-switch" role="switch" checked={includeGracePeriod} disabled={action !== "idle" || !hasPlatformCapabilities()} onChange={event => setIncludeGracePeriod(event.target.checked)} type="checkbox" /></SettingRow>
      {(action === "scan" || action === "clean") && <SettingRow title="当前任务" description="请求停止后，已完成的原子步骤保持一致。"><button onClick={() => void cancel()} type="button">取消</button></SettingRow>}
    </SettingGroup></div>
    <div className="settings-panel-gap"><SettingGroup label="历史与状态">
      <SettingRow title={<span className="settings-title-with-hint">笔记历史 <SettingsHint label="历史版本保留规则">未固定的自动保存历史保留 14 天；手动、导入、恢复和结构历史保留 30 天。固定版本不会自动裁剪。</SettingsHint></span>} description="查看和管理保留的历史版本。"><button onClick={() => window.dispatchEvent(new Event("research-assistant:open-notes"))} type="button">前往笔记历史</button></SettingRow>
      <SettingRow title="最近清理"><span>{report?.lastSweepAt ? new Date(report.lastSweepAt).toLocaleString("zh-CN") : "尚未清理"}</span></SettingRow>
    </SettingGroup></div>
    {result !== undefined && result.prunedRevisionCount > 0 && <p className="storage-footnote">本次按保留策略裁剪了 {result.prunedRevisionCount.toLocaleString("zh-CN")} 条未固定历史版本。</p>}
  </section>;
}
