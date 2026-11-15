import {systemDocumentPath, type SystemDocument, type SystemDocumentGateway} from "@research-assistant/core/agent";
import {openTextDocument, type TextDocumentRequest} from "../../components/text-editor/textDocumentEvents";
import {systemDocumentGateway} from "../../services/systemDocumentGateway";

/** Resolve the current object before requesting any UI. Never open a catalog as
 * a fallback for an unresolved tag. Persistence remains owned by the gateway. */
export async function openSystemDocument(id: string, gateway: SystemDocumentGateway = systemDocumentGateway,
  active: () => boolean = () => true): Promise<void> {
  let current: SystemDocument;
  try {
    current = await gateway.read(id);
    if (current.id !== id) throw new Error("Object identity mismatch");
  } catch { throw new Error("找不到文件对象"); }
  openResolvedSystemDocument(current, gateway, active);
}

/** Navigation already resolved this exact object; reuse it instead of reading twice. */
export function openResolvedSystemDocument(document: SystemDocument, gateway: SystemDocumentGateway = systemDocumentGateway, active: () => boolean = () => true): void {
  if (!active()) return;
  if (!openTextDocument(systemTextDocument(document,gateway))) throw new Error("无法打开文本标签页。");
}

export function systemTextDocument(document:SystemDocument,gateway:SystemDocumentGateway=systemDocumentGateway):TextDocumentRequest {
  let current = document;
  if (systemDocumentPath(current.id) !== current.path) throw new Error("找不到文件对象");
  return {
    reopen:{kind:"system",id:current.id},
    id: `system:${current.id}`, path: systemDocumentPath(current.id) ?? current.path, title: current.name,
    content: current.content, baseline: current.baseline,
    save: async (content, baseline) => {
      current = await gateway.write({id: current.id, expectedRevision: current.revision, content, baseline});
    },
  };
}
