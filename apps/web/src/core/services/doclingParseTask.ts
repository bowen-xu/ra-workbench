import { createElement } from "react";
import { Square } from "lucide-react";
import { DoclingNotificationCaption, type DoclingNotificationDocument } from "../components/DoclingNotificationCaption";
import type { DoclingParserGateway, DoclingParseRunDetail, DoclingParseTaskStatus } from "../contracts/docling";
import { doclingErrorMessage } from "./doclingError";
import { notify, type NotificationAction } from "./notifications";

export function doclingTaskMessage(task: DoclingParseTaskStatus): string {
  if (task.state === "cancelling") return "正在终止解析…";
  const labels: Record<string, string> = {
    preparing: "正在准备 PDF 解析…", connecting: "正在连接 Docling 服务…",
    starting_service: "正在启动服务并加载模型…", reading_file: "正在读取 PDF 文件…",
    queued: "等待解析资源…", parsing: "正在解析 PDF…", exporting: "正在整理结构化结果…", saving: "正在保存解析结果…",
  };
  if (task.stage === "parsing" && task.totalPages > 0) return `正在解析 PDF · ${task.completedPages}/${task.totalPages} 页`;
  return labels[task.stage] ?? "正在解析 PDF…";
}
/** Survives panel navigation: one task owns one updatable, recoverable global message. */
export async function runDoclingParse(gateway: DoclingParserGateway, itemId: string, attachmentId: string, document: DoclingNotificationDocument, force = true): Promise<DoclingParseRunDetail | undefined> {
  const description = createElement(DoclingNotificationCaption, document);
  const id = notify.loading("正在准备 PDF 解析…", { description });
  let task: DoclingParseTaskStatus | undefined;
  let stopping = false;
  let finished = false;
  const actions = (): readonly NotificationAction[] => [{ id: "stop", label: stopping ? "正在终止解析" : "终止解析", icon: createElement(Square, { "aria-hidden": true }), disabled: stopping,
    onClick: async () => {
      if (!task || stopping || finished) return;
      stopping = true;
      notify.update(id, { message: "正在终止解析…", actions: actions() });
      try { await gateway.cancelParseTask(task.id); }
      catch (reason) {
        stopping = false;
        notify.update(id, { message: "终止请求失败，解析仍在进行", actions: actions() });
        throw new Error(doclingErrorMessage(reason));
      }
    } }];
  try {
    task = await gateway.startParseTask(itemId, attachmentId, { force });
    while (true) {
      if (task.state === "succeeded") {
        if (!task.parseRunId) throw new Error("Docling 返回的解析任务缺少结果标识");
        const detail = await gateway.getParseRun(task.parseRunId);
        finished = true;
        notify.success("PDF 解析完成", { id, description });
        return detail;
      }
      if (task.state === "cancelled") { finished = true; notify.info("PDF 解析已终止", { id, description }); return; }
      if (task.state === "failed") throw task.error ?? new Error("PDF 解析失败");
      notify.update(id, { message: stopping ? "正在终止解析…" : doclingTaskMessage(task), actions: actions() });
      await new Promise(resolve => setTimeout(resolve, 300));
      task = await gateway.getParseTask(task.id);
    }
  } catch (reason) {
    if (task && !["failed", "cancelled", "succeeded"].includes(task.state)) {
      await gateway.cancelParseTask(task.id).catch(() => undefined);
    }
    finished = true;
    notify.error(doclingErrorMessage(reason), { id, description });
    throw reason;
  }
}
