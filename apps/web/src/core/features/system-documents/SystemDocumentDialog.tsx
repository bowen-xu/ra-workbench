import { Dialog } from "../../components/overlay/OverlayDialog";
import { DialogHeading } from "@ariakit/react";
import { Copy, ExternalLink, FileText, X } from "lucide-react";
import { useEffect, useState } from "react";
import type { SystemDocument, SystemDocumentGateway } from "@research-assistant/core/agent";
import { openSystemDocument } from "./openSystemDocument";
import { systemDocumentGateway } from "../../services/systemDocumentGateway";
import { notify } from "../../services/notifications";
import { copySystemDocument } from "./copySystemDocument";
import "./SystemDocumentDialog.css";

const descriptions = {
  portrait: "文献画像使用的指令与格式",
  reading: "结构化阅读使用的指令与格式",
  shared: "画像与阅读共用的技能与参考文档",
};

export function SystemDocumentDialog({ id, workflow, onClose, gateway = systemDocumentGateway }: {
  readonly id?: string;
  readonly workflow?: "portrait" | "reading";
  readonly onClose: () => void;
  readonly gateway?: SystemDocumentGateway;
}) {
  const [catalog, setCatalog] = useState<readonly SystemDocument[]>([]);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);
  const [opening, setOpening] = useState<string>();
  useEffect(() => {
    let active = true;
    setLoading(true); setFailed(false);
    void gateway.list().then(rows => { if (active) setCatalog(rows); })
      .catch(reason => { if (active) { setFailed(true); notify.error(String(reason)); } })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [gateway]);

  async function open(entry: SystemDocument): Promise<void> {
    if (opening) return;
    setOpening(entry.id);
    try {
      await openSystemDocument(entry.id, gateway);
      onClose();
    } catch (reason) { notify.error(reason instanceof Error ? reason.message : "找不到文件对象"); }
    finally { setOpening(undefined); }
  }
  const visible = catalog.filter(document => document.id === id || !workflow || document.workflow === workflow || document.workflow === "shared");
  return <Dialog open portal className="system-document-dialog" aria-label="内置指令文档" hideOnInteractOutside={false} hideOnEscape={() => { onClose(); return false; }}>
    <header><div><DialogHeading render={<h2 />}>内置指令文档</DialogHeading><p>在独立标签页中编辑文档，保存后立即用于后续任务。</p></div><button type="button" aria-label="关闭内置文档" onClick={onClose}><X size={18} /></button></header>
    <div className="system-document-list" aria-label="内置文档列表">
      {loading ? <p role="status">正在读取文档列表…</p> : failed ? <p role="alert">无法读取内置文档，请关闭后重试。</p> : visible.length === 0 ? <p role="status">暂无内置文档。</p> : visible.map(document => <div className="system-document-row" key={document.id}>
        <span className="system-document-icon"><FileText size={21} strokeWidth={1.6} aria-hidden="true" /></span>
        <div className="system-document-copy">
          <span className="system-document-name">{document.name}</span>
          <span className="system-document-description">{descriptions[document.workflow]}</span>
        </div>
        <button type="button" aria-label={`复制 ${document.name} 路径`} onClick={() => void copySystemDocument(document).then(ok => ok ? notify.success("已复制文件引用，可粘贴到聊天输入框。") : notify.error("复制失败，请重试。"))}><Copy size={15} /></button>
        <button type="button" disabled={opening !== undefined} aria-label={`编辑 ${document.name}`} onClick={() => void open(document)}><ExternalLink size={15} />{opening === document.id ? "打开中…" : "编辑"}</button>
      </div>)}
    </div>
  </Dialog>;
}
