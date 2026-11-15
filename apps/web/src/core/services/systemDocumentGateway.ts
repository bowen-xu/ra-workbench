import {systemDocumentPath, type SystemDocument, type SystemDocumentApproval, type SystemDocumentGateway, type SystemDocumentWrite} from "@research-assistant/core/agent";
import { invoke } from "../runtime/platformCore";
import {hasPlatformCapabilities} from "../runtime/appRuntime";
async function documentInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    if (!hasPlatformCapabilities()) throw new Error("找不到文件对象");
    return invoke<T>(command, args);
}
function text(value: unknown, max = 262144): string { if (typeof value !== "string" || value.includes("\0") || new TextEncoder().encode(value).length > max)
    throw new Error("内置文档响应无效"); return value; }
function object(value: unknown): Record<string, unknown> { if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("内置文档响应无效"); return value as Record<string, unknown>; }
export function decodeSystemDocument(value: unknown): SystemDocument {
    const row = object(value), id = text(row.id, 128), workflow = text(row.workflow, 32), path = text(row.path, 512);
    const canonical = systemDocumentPath(id);
    if (!/^[a-z][a-z0-9-]*$/.test(id) || !canonical || path !== canonical || !["portrait", "reading", "shared"].includes(workflow))
        throw new Error("内置文档身份无效");
    return { id, name: text(row.name, 256), workflow: workflow as SystemDocument["workflow"], path: canonical, content: text(row.content), baseline: text(row.baseline), revision: text(row.revision, 128) };
}
export const systemDocumentGateway: SystemDocumentGateway = {
    async list() { const rows = await documentInvoke<unknown>("system_documents_list"); if (!Array.isArray(rows) || rows.length > 32)
        throw new Error("内置文档目录无效"); return rows.map(decodeSystemDocument); },
    async read(id) { return decodeSystemDocument(await documentInvoke("system_documents_read", { id })); },
    async write(input: SystemDocumentWrite) { return decodeSystemDocument(await documentInvoke("system_documents_write", { input })); },
    async pending(conversationId) { const rows = await documentInvoke<unknown>("system_documents_pending", { conversationId }); if (!Array.isArray(rows) || rows.length > 16)
        throw new Error("写入授权列表无效"); return rows.map(value => { const row = object(value); const request: SystemDocumentApproval = { id: text(row.id, 128), conversationId: text(row.conversationId, 128), documentId: text(row.documentId, 128), name: text(row.name, 256), path: text(row.path, 512), before: text(row.before), after: text(row.after) }; if (request.conversationId !== conversationId || request.path !== systemDocumentPath(request.documentId))
        throw new Error("写入授权身份无效"); return request; }); },
    async decide(conversationId, requestId, allow) { await documentInvoke("system_documents_decide", { conversationId, requestId, allow }); },
};
