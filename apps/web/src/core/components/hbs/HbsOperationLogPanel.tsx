import { useState, useSyncExternalStore } from "react";
import { HbsOperationLog, type HbsOperationEntry } from "./operationLog";
import {useOperationLogExport} from "./useOperationLogExport";
import "./HbsOperationLog.css";

function Entry({entry}: {readonly entry: HbsOperationEntry}): React.JSX.Element {
  const [open, setOpen] = useState(false);
  return <li value={entry.sequence}><time dateTime={entry.time}>{new Date(entry.time).toLocaleTimeString()}</time> · {entry.label}
    <details onToggle={event => setOpen(event.currentTarget.open)}><summary>操作数据</summary>{open && <pre>{JSON.stringify(entry.detail, null, 2)}</pre>}</details>
  </li>;
}
export function HbsOperationLogPanel({log, initiallyOpen = false, onClear, retentionDescription = "成功重置或刷新后清空，重置失败时保留。"}: {
  readonly log: HbsOperationLog;
  readonly initiallyOpen?: boolean;
  readonly onClear?: () => void;
  readonly retentionDescription?: string;
}): React.JSX.Element {
  const snapshot = useSyncExternalStore(log.subscribe, log.getSnapshot);
  const entries = snapshot.filter(entry => entry.channel === "operation");
  const [open, setOpen] = useState(initiallyOpen);
  const {copy, download, copying, downloading} = useOperationLogExport(log);
  const stats = log.getStats();
  return <details className="hbs-operation-log" open={open} onToggle={event => {if (event.target === event.currentTarget) setOpen(event.currentTarget.open);}}>
    <summary>操作记录（{entries.length}）</summary>
    <p>仅记录记录器启用后的操作；操作触发、原生事务和模型结果分别记录。日志可能包含笔记正文，只保留在本机内存，不自动落盘或上传；显式复制或导出才写入剪贴板或文件。{retentionDescription}JSON 包含操作、模型状态及通信记录。</p>
    <p>保留 {stats.retained} 条／共 {stats.recorded} 条，{stats.bytes} 字节；最多 {stats.limits.maxEntries} 条／{stats.limits.maxBytes} 字节，单条详情最多 {stats.limits.maxDetailBytes} 字节。{!stats.complete && <strong>已丢弃 {stats.dropped} 条，省略 {stats.truncated} 条详情；记录不完整，不能据此完整重放。</strong>}</p>
    <div className="hbs-operation-log-actions">
      <button type="button" disabled={copying} onClick={() => { void copy(); }}>{stats.complete ? "复制完整操作记录" : "复制现有操作记录"}</button>
      <button type="button" disabled={downloading} onClick={() => {void download();}}>导出操作记录 JSON</button>
      {onClear && <button type="button" onClick={onClear}>清空操作记录</button>}
    </div>
    {open && <ol>{entries.map(entry => <Entry key={entry.sequence} entry={entry} />)}</ol>}
  </details>;
}
