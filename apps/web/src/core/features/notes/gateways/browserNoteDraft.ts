import {createEmptyNoteDocument, decodeNoteDocument, normalizeNoteTitle, notePageId, noteBlockId} from "@research-assistant/core/notes";
import type {NoteDraftCommit, NoteDraftReceipt} from "@research-assistant/data/notes";
import {createNoteDocumentAdapter} from "@research-assistant/core/hbs";
import {directTextDiff} from "../../../components/hbs/codeMirrorHbsPolicy";
import {readState, writeState, now} from "./browserNotesStorage";
import {saveBrowserDocument} from "./browserNotesGateway";
import {withBrowserNotesWrite} from "./browserNotesWrites";

const adapter = createNoteDocumentAdapter(directTextDiff);
export function publishBrowserNoteDraft(id: string, request: NoteDraftCommit): Promise<NoteDraftReceipt> {
  return withBrowserNotesWrite(async () => {
    const input = request.commit;
    const fail = (message: string): never => {throw Object.assign(new Error(`MODEL_INVALID_REQUEST：${message}`), {code: "MODEL_INVALID_REQUEST"});};
    if (!input.id || input.id.length > 256 || !Number.isSafeInteger(input.expectedVersion) || input.expectedVersion < 0) fail("新建页面请求或版本无效");
    const saved = decodeNoteDocument(input.saved), baseline = decodeNoteDocument(input.baseline);
    if (saved.pageId !== id || baseline.pageId !== id || request.source?.document.pageId === id) fail("新建页面身份不匹配");
    let state = await readState({hbsPageIds:[id,...(request.source ? [request.source.document.pageId] : [])]});
    const prior = state.hbsRecords?.[id]?.receipts?.[input.id];
    if (prior) {
      if (JSON.stringify(prior.creation) !== JSON.stringify({favorite: request.favorite, source: request.source})
        || prior.request.expectedVersion !== input.expectedVersion || !adapter.equal(prior.request.saved,saved) || !adapter.equal(prior.request.baseline,baseline)) fail("新建页面请求身份已用于其他内容");
      return structuredClone(prior.reply) as NoteDraftReceipt;
    }
    if (state.pages.some(page => page.id === id)) fail("页面身份已存在，新建操作未覆盖原页面");
    const pageId = notePageId(id), timestamp = now();
    const page = {id:pageId,title:normalizeNoteTitle(saved.title ?? ""),schemaVersion:1 as const,revision:input.expectedVersion,
      position:state.pages.filter(page => !page.parentPageId).length,status:"active" as const,isFavorite:request.favorite,
      projectIds:[],tagIds:[],createdAt:timestamp,updatedAt:timestamp};
    state = {...state,pages:[...state.pages,page],documents:{...state.documents,[id]:createEmptyNoteDocument(pageId,noteBlockId(crypto.randomUUID()))},
      ...(request.favorite ? {favoriteOrder:[pageId,...state.favoriteOrder ?? []]} : {})};
    state = saveBrowserDocument(state,{document:saved,expectedRevision:input.expectedVersion,saveReason:"manual"});
    if (request.source) {
      const sourceId = request.source.document.pageId, before = state.documents[sourceId];
      if (!before || state.pages.find(page=>page.id===sourceId)?.status !== "active") fail("来源页面不存在或已停用");
      const priorBaseline = state.hbsRecords?.[sourceId]?.baseline;
      if (priorBaseline && !adapter.equal(priorBaseline,before!)) fail("请先完成来源笔记的修改审阅");
      state = saveBrowserDocument(state,request.source);
      state = {...state,hbsRecords:{...state.hbsRecords,[sourceId]:{...state.hbsRecords?.[sourceId],baseline:state.documents[sourceId]!}}};
    }
    const published = {page:state.pages.find(page=>page.id===id)!,document:state.documents[id]!};
    const source = request.source ? {page:state.pages.find(page=>page.id===request.source!.document.pageId)!,document:state.documents[request.source.document.pageId]!} : undefined;
    const reply: NoteDraftReceipt = {type:"receipt",requestId:input.id,snapshot:{version:published.page.revision,saved:published.document,baseline},page:published,...(source ? {source} : {})};
    state = {...state,hbsRecords:{...state.hbsRecords,[id]:{baseline,receipts:{[input.id]:{request:input,reply,creation:{favorite:request.favorite,...(request.source ? {source:request.source} : {})}}}}}};
    // Canonical page, first body/baseline, source link and exact retry receipt
    // are one IDB commit; failure publishes none of them.
    await writeState(state);
    return reply;
  });
}
