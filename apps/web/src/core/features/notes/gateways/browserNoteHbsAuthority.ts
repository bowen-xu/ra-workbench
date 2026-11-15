import {createNoteDocumentAdapter, type DocumentCommit, type DocumentSnapshot} from "@research-assistant/core/hbs";
import {decodeDocumentCheckpoint} from "@research-assistant/data/hbs";
import {pageEntryTargets,type NoteDocumentV1} from "@research-assistant/core/notes";
import {directTextDiff} from "../../../components/hbs/codeMirrorHbsPolicy";
import type {NoteDocumentSaveRequest, NoteDocumentSnapshot} from "@research-assistant/data/notes";
import {saveBrowserDocument} from "./browserNotesGateway";
import {now, readState, readNoteHbsState, writeNoteHbsRecord, writeState, type BrowserNotesState} from "./browserNotesStorage";
import type {BrowserNoteHbsRecord} from "./browserNoteHbsState";
import {withBrowserNotesWrite} from "./browserNotesWrites";
import {getAppStorage} from "../../../runtime/appStorage";
import {changedPageEntryTargets,pageEntryReadPages} from "./browserPageEntryLifecycle";

const adapter = createNoteDocumentAdapter(directTextDiff);
function fail(code: string, message: string): never { throw Object.assign(new Error(`${code}：${message}`), {code}); }
function document(value: unknown, id: string): NoteDocumentV1 {
  let decoded: NoteDocumentV1;
  try { decoded = adapter.decode(value); }
  catch (reason) { fail("MODEL_INVALID_DOCUMENT", reason instanceof Error ? reason.message : "笔记结构无效。"); }
  if (decoded.pageId !== id) fail("MODEL_INVALID_DOCUMENT", "不能跨页面写入笔记。");
  return decoded;
}

/** Browser platform adapter for the full App preview. It reuses the actual
 * browser note repository/transaction; only native IPC is replaced. */
export function createBrowserNoteHbsAuthority(options: {
  readonly legacyBaseline?: (id: string) => NoteDocumentV1 | undefined;
  readonly acknowledge?: (snapshot: DocumentSnapshot<NoteDocumentV1>) => void;
} = {}) {
  const owner = getAppStorage();
  function checkOwner(): void { if (getAppStorage() !== owner) fail("MODEL_OWNER_CHANGED", "笔记存储拥有者已切换，原操作未执行。"); }
  function snapshot(state: BrowserNotesState, id: string): DocumentSnapshot<NoteDocumentV1> {
    const page = state.pages.find(page => page.id === id), body = state.documents[id];
    if (!page || !body) fail("MODEL_NOT_FOUND", "找不到笔记。");
    const saved = {...body, title: body.title ?? page.title}, legacy = options.legacyBaseline?.(id);
    const baseline = state.hbsRecords?.[id]?.baseline ?? legacy ?? saved;
    return {version: page.revision, saved, baseline: document({...baseline, title: baseline.title ?? legacy?.title ?? page.title}, id)};
  }
  function record(state: BrowserNotesState, id: string, value: BrowserNoteHbsRecord): BrowserNotesState {
    return {...state, hbsRecords: {...state.hbsRecords, [id]: value}};
  }
  // Formal saves still preserve hierarchy validation and history retention;
  // their metadata/history work is separate from opening or preserving H.
  const point = (id: string) => { checkOwner(); return readNoteHbsState(id); };
  const loadForWrite = async(id:string,document:NoteDocumentV1)=>{
    checkOwner();
    const before=await readState({hbsPageId:id,pageIds:[id],revisions:true});
    const targets=new Set(changedPageEntryTargets(before.documents[id]!,document));
    if(!targets.size)return before;
    for(let depth=0;depth<=64;depth++){
      const state=await readState({hbsPageId:id,pageIds:pageEntryReadPages(id,targets),revisions:true});
      let added=false;
      for(const target of [...targets])for(const child of state.documents[target]?pageEntryTargets(state.documents[target]!):[])
        if(!targets.has(child)){targets.add(child);added=true;}
      if(!added)return state;
      if(targets.size>20_000)break;
    }
    return fail("MODEL_INVALID_DOCUMENT","页面入口结构超过安全限制。");
  };
  async function read(id: string): Promise<DocumentSnapshot<NoteDocumentV1>> { return snapshot(await point(id), id); }
  async function commit(id: string, input: DocumentCommit<unknown>) {
    if (!input || typeof input.id !== "string" || !input.id.trim() || !Number.isSafeInteger(input.expectedVersion) || input.expectedVersion < 0)
      fail("MODEL_INVALID_REQUEST", "保存请求身份或修订号无效。");
    const request: DocumentCommit<NoteDocumentV1> = {...input, saved: document(input.saved, id), baseline: document(input.baseline, id)};
    return withBrowserNotesWrite(async () => {
      let state = await loadForWrite(id,request.saved);
      const previous = state.hbsRecords?.[id] ?? {}, prior = previous.receipts?.[request.id];
      if (prior) {
        if (prior.request.expectedVersion !== request.expectedVersion || !adapter.equal(prior.request.saved, request.saved) || !adapter.equal(prior.request.baseline, request.baseline))
          fail("MODEL_INVALID_REQUEST", `请求 ${request.id} 已用于不同内容。`);
        return structuredClone(prior.reply);
      }
      const current = snapshot(state, id);
      if (current.version !== request.expectedVersion) return {type: "conflict" as const, requestId: request.id, snapshot: current};
      const page = state.pages.find(page => page.id === id)!;
      if (page.status !== "active") fail("MODEL_RESOURCE_INACTIVE", "非活动笔记不能提交正文或审阅决定。");
      const saved = {...request.saved, title: request.saved.title ?? current.saved.title!};
      const baseline = {...request.baseline, title: request.baseline.title ?? current.baseline.title!};
      if (!adapter.equal(current.saved, saved)) state = saveBrowserDocument(state, {
        document: saved, expectedRevision: request.expectedVersion, saveReason: "manual",
      });
      else state = {...state, pages: state.pages.map(value => value.id === id ? {...value, revision: value.revision + 1, updatedAt: now()} : value)};
      state = record(state, id, {...previous, baseline});
      const after = snapshot(state, id), reply = {type: "receipt" as const, requestId: request.id, snapshot: after};
      state = record(state, id, {...state.hbsRecords![id], receipts: {...previous.receipts, [request.id]: {request, reply}}});
      // S, B and the exact dedup result commit once. A failed IDB transaction
      // cannot publish a receipt or a partial saved document.
      await writeState(state); options.acknowledge?.(after); return reply;
    });
  }
  async function readPreservation(id: string) {
    const state = await point(id); snapshot(state, id);
    const value = state.hbsRecords?.[id]?.checkpoint;
    return value ? decodeDocumentCheckpoint(value, adapter) : null;
  }
  async function savePreservation(id: string, input: unknown) {
    const checkpoint = decodeDocumentCheckpoint(input, adapter);
    for (const value of [checkpoint.content, checkpoint.ancestor, checkpoint.baselineAncestor, checkpoint.observed.saved, checkpoint.observed.baseline,
      ...(checkpoint.reviewAncestor ? [checkpoint.reviewAncestor] : [])]) document(value, id);
    return withBrowserNotesWrite(async () => {
      const state = await point(id); snapshot(state, id);
      const previous = state.hbsRecords?.[id] ?? {}, revision = (previous.preservationRevision ?? 0) + 1;
      await writeNoteHbsRecord(state,id,{...previous,checkpoint,preservationRevision:revision});
      return {revision};
    });
  }
  /** Agent fixture writes use the same formal save algorithm and atomically
   * install their review baseline before the mounted HBS session reads them. */
  async function saveExternal(request: NoteDocumentSaveRequest, baseline?: NoteDocumentV1): Promise<NoteDocumentSnapshot> {
    const id = request.document.pageId;
    return withBrowserNotesWrite(async () => {
      let state = await loadForWrite(id,request.document); const before = snapshot(state, id);
      if (before.version !== request.expectedRevision) fail("MODEL_VERSION_CONFLICT", "笔记已被其他操作更新。");
      if (state.pages.find(page => page.id === id)!.status !== "active") fail("MODEL_RESOURCE_INACTIVE", "非活动笔记不能提交正文或审阅决定。");
      if (!adapter.equal(before.saved, request.document)) state = saveBrowserDocument(state, request);
      else if (baseline && !adapter.equal(before.baseline, baseline)) state = {...state, pages: state.pages.map(page => page.id === id
        ? {...page, revision: page.revision + 1, updatedAt: now()} : page)};
      else return {page: state.pages.find(page => page.id === id)!, document: state.documents[id]!};
      if (baseline) state = record(state, id, {...state.hbsRecords?.[id], baseline: document(baseline, id)});
      await writeState(state);
      return {page: state.pages.find(page => page.id === id)!, document: state.documents[id]!};
    });
  }
  return {read, commit, readPreservation, savePreservation, saveExternal};
}
