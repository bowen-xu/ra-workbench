import {useCallback, useRef, useState} from "react";
import {saveNoteRecoveryFile} from "../../features/notes/transfer/saveNoteRecoveryFile";
import {notify} from "../../services/notifications";
import type {HbsOperationLog} from "./operationLog";

/** Both the adapter panel and production menu export the recorder's same JSON. */
export function useOperationLogExport(log: HbsOperationLog) {
  const busy = useRef({copy: false, download: false});
  const [copying, setCopying] = useState(false), [downloading, setDownloading] = useState(false);
  const copy = useCallback(async (): Promise<void> => {
    if (busy.current.copy) return;
    busy.current.copy = true; setCopying(true);
    try {
      const value = log.exportJson(), complete = log.getStats().complete;
      await navigator.clipboard.writeText(value);
      notify.success(complete ? "已复制完整操作记录" : "已复制现有操作记录；丢弃或省略情况已注明。");
    } catch (reason) {
      notify.error(`MODEL_OPERATION_LOG_COPY_FAILED：${reason instanceof Error ? reason.message : String(reason)}`);
    } finally {busy.current.copy = false; setCopying(false);}
  }, [log]);
  const download = useCallback(async (): Promise<void> => {
    if (busy.current.download) return;
    busy.current.download = true; setDownloading(true);
    try {
      const result = await saveNoteRecoveryFile(`hbs-operations-${new Date().toISOString().replace(/[:.]/g, "-")}.json`, JSON.parse(log.exportJson()), {title: "导出笔记操作记录"});
      if (result !== "cancelled") notify.success(result === "saved" ? "已导出操作记录。" : "已发起操作记录下载。");
    } catch (reason) {
      notify.error(`MODEL_OPERATION_LOG_EXPORT_FAILED：${reason instanceof Error ? reason.message : String(reason)}`);
    } finally {busy.current.download = false; setDownloading(false);}
  }, [log]);
  return {copy, download, copying, downloading};
}
