import {createEmptyNoteDocument, decodeNoteDocument, noteBlockId, notePageId, normalizeNoteTitle, type NoteDocumentV1, type NotePageId, type NoteBlockV1, type NotePage} from "@research-assistant/core/notes";
import type {DocumentCheckpoint, DocumentCommit, DocumentSnapshot} from "@research-assistant/core/hbs";
import {decodeDocumentCheckpoint} from "@research-assistant/data/hbs";
import {createNoteDocumentAdapter} from "@research-assistant/core/hbs";
import type {NoteDocumentSnapshot, NoteDraftCommit} from "@research-assistant/data/notes";
import type {NotesClient} from "../NotesClient";
import {getHbsDocumentGateway, type AppHbsDocumentGateway} from "../../../services/hbsDocumentGateway";
import {getAppStorage} from "../../../runtime/appStorage";
import {flushStorage} from "../../../runtime/storagePersistence";
import {clearRecoveryDraft, createRecoveryDraft, loadRecoveryDraft, saveRecoveryDraft} from "../../../services/recoveryDraftStore";
import {directTextDiff} from "../../../components/hbs/codeMirrorHbsPolicy";
import {flushNoteReview, holdTextEditor} from "../editor/noteTextEditorPorts";
import {prepareNavigationSessions} from "./noteNavigationSessions";
import {publishNoteDraft} from "./noteDraftTransport";
import {hasNoteDraftContent} from "@research-assistant/core/notes";
import {notify} from "../../../services/notifications";
import {markNoteDraftIdentity,noteDraftIdentity as identity} from "./noteDraftIdentity";

export type { NotePageSource } from "../../../contracts/noteResources";
import type { NotePageSource } from "../../../contracts/noteResources";
interface Draft {
  readonly initial: NoteDocumentSnapshot;
  readonly source?: NotePageSource;
  authority: DocumentSnapshot<NoteDocumentV1>;
  checkpoint?: DocumentCheckpoint<NoteDocumentV1>;
  creation?: NoteDraftCommit;
  materialized?: NoteDocumentSnapshot;
}
const adapter = createNoteDocumentAdapter(directTextDiff);

/** Creates a canonical page before opening an editor. The existing atomic
 * publisher owns page/B/favorite/source writes and exact retries. Legacy
 * recovery readers below preserve input from older unpublished sessions. */
export class NotePages {
  private readonly entries = new Map<string,Draft>();
  private pendingCreation:{readonly id:NotePageId;readonly entry:Draft;readonly commit:DocumentCommit<NoteDocumentV1>}|undefined;
  private inFlight:Promise<NoteDocumentSnapshot>|undefined;
  private readonly preparations = new Map<string,{readonly port:NonNullable<ReturnType<typeof prepareNavigationSessions>>;dispatched:boolean}>();
  constructor(private readonly client: NotesClient) {}

  async create(favorite = false, source?: NotePageSource): Promise<NoteDocumentSnapshot> {
    if(this.inFlight)return this.inFlight;
    if(this.pendingCreation){
      const {id,entry,commit}=this.pendingCreation;
      throw Object.assign(new Error(`MODEL_REQUEST_UNSETTLED：请先重试未核对的新建请求（请求 ${commit.id}）`),{retryCreation:()=>this.runCreation(id,entry,commit)});
    }
    const id = notePageId(crypto.randomUUID()), now = new Date().toISOString();
    const document = {...createEmptyNoteDocument(id,noteBlockId(crypto.randomUUID())),title:""};
    const initial: NoteDocumentSnapshot = {page:{id,title:"无标题",schemaVersion:1,revision:0,position:0,status:"active",isFavorite:favorite,projectIds:[],tagIds:[],createdAt:now,updatedAt:now},document};
    const entry:Draft={initial,authority:{version:0,saved:document,baseline:document},...(source ? {source} : {})};
    this.entries.set(id,entry);
    const commit={id:crypto.randomUUID(),expectedVersion:0,saved:document,baseline:document};
    this.pendingCreation={id,entry,commit};return this.runCreation(id,entry,commit);
  }
  private runCreation(id:NotePageId,entry:Draft,commit:DocumentCommit<NoteDocumentV1>):Promise<NoteDocumentSnapshot>{
    if(this.inFlight)return this.inFlight;
    this.inFlight=this.completeCreation(id,entry,commit).finally(()=>{this.inFlight=undefined;});
    return this.inFlight;
  }
  private async completeCreation(id:NotePageId,entry:Draft,commit:DocumentCommit<NoteDocumentV1>):Promise<NoteDocumentSnapshot>{
    try{
      const receipt=await this.publish(id,entry,commit,()=>{});
      this.pendingCreation=undefined;
      this.client.notifyChanged({kind:"document",pageId:id,revision:receipt.page.page.revision});
      if(receipt.source)this.client.notifyChanged({kind:"document",pageId:receipt.source.page.id,revision:receipt.source.page.revision});
      return receipt.page;
    }catch(reason){
      // Keep exact uncertain creation requests recoverable; no temporary page
      // is mounted while a canonical write is pending or has failed.
      const error=reason instanceof Error?reason:new Error(String(reason));
      if(entry.creation)Object.assign(error,{retryCreation:()=>this.runCreation(id,entry,commit)});
      else this.pendingCreation=undefined;
      throw error;
    }
  }
  async observeStored(snapshot:NoteDocumentSnapshot):Promise<void>{
    const entry=this.get(snapshot.page.id);if(!entry||entry.materialized)return;
    // Migrate an older unpublished recovery checkpoint into the existing HBS
    // preservation slot before releasing that old record; never drop its input.
    if(entry.checkpoint)await getHbsDocumentGateway().savePreservation("note",snapshot.page.id,entry.checkpoint);
    entry.materialized=snapshot;markNoteDraftIdentity(snapshot.page.id,false);
    clearRecoveryDraft(identity(snapshot.page.id));await flushStorage(getAppStorage());
  }
  private get(id: string): Draft | undefined {
    const current = this.entries.get(id);
    if (current) return current;
    const stored = loadRecoveryDraft(identity(id));
    if (!stored) return;
    const value = stored.payload as Draft;
    if (value.initial?.page.id !== id || decodeNoteDocument(value.initial.document).pageId !== id
      || decodeNoteDocument(value.authority?.saved).pageId !== id || decodeNoteDocument(value.authority.baseline).pageId !== id
      || !Number.isSafeInteger(value.authority.version)) throw new Error("MODEL_RECOVERY_INVALID：新笔记草稿损坏，原记录已保留");
    if (value.checkpoint) value.checkpoint = decodeDocumentCheckpoint(value.checkpoint,adapter);
    this.entries.set(id,value); return value;
  }
  isDraft(id: string): boolean {const entry=this.get(id);return !!entry && !entry.materialized;}
  snapshot(id: string): NoteDocumentSnapshot | undefined {
    const entry = this.get(id);
    return entry && !entry.materialized ? {...entry.initial,page:{...entry.initial.page,revision:entry.authority.version},document:entry.authority.saved} : undefined;
  }
  isEmpty(id: string): boolean {const entry=this.get(id);return !!entry && !entry.materialized && !entry.creation && !hasNoteDraftContent(entry.checkpoint?.content ?? entry.authority.saved);}
  forgetEmpty(id: string): void {
    const entry=this.entries.get(id);
    if (entry && !entry.materialized && !entry.creation && !hasNoteDraftContent(entry.checkpoint?.content ?? entry.authority.saved)) this.entries.delete(id);
  }
  private async preserve(id: string, entry: Draft): Promise<void> {
    if (entry.creation || hasNoteDraftContent(entry.checkpoint?.content ?? entry.authority.saved)) {
      saveRecoveryDraft(createRecoveryDraft(identity(id),entry));
      await flushStorage(getAppStorage());
    } else if(loadRecoveryDraft(identity(id))) {
      clearRecoveryDraft(identity(id));await flushStorage(getAppStorage());
    }
  }
  gateway(id: NotePageId, metadata: (page: NotePage) => void): AppHbsDocumentGateway | undefined {
    const entry=this.get(id); if (!entry || entry.materialized) return;
    const real=getHbsDocumentGateway();
    return {
      read: async (kind,key) => entry.materialized ? real.read(kind,key) : structuredClone(entry.authority),
      readPreservation: async (kind,key) => entry.materialized ? await real.readPreservation(kind,key) ?? entry.checkpoint ?? null : entry.checkpoint ?? null,
      savePreservation: async (kind,key,value) => {
        if (entry.materialized) {
          const result=await real.savePreservation(kind,key,value);
          clearRecoveryDraft(identity(id));await flushStorage(getAppStorage());return result;
        }
        entry.checkpoint=decodeDocumentCheckpoint(value,adapter); await this.preserve(id,entry); return {revision:1};
      },
      commit: async (kind,key,value) => {
        if (entry.materialized && entry.creation?.commit.id !== value.id) return real.commit(kind,key,value);
        const commit: DocumentCommit<NoteDocumentV1> = {...value,saved:decodeNoteDocument(value.saved),baseline:decodeNoteDocument(value.baseline)};
        if (!hasNoteDraftContent(commit.saved)) {
          entry.authority={version:commit.expectedVersion+1,saved:commit.saved,baseline:commit.baseline};
          return {type:"receipt",requestId:commit.id,snapshot:entry.authority};
        }
        return this.publish(id,entry,commit,metadata);
      },
    };
  }
  private async publish(id: NotePageId, entry: Draft, commit: DocumentCommit<NoteDocumentV1>, metadata: (page: NotePage) => void) {
    let preparation=this.preparations.get(id);
    let prepared=preparation?.port;
    let dispatched=preparation?.dispatched ?? false;
    const sourceId=entry.source?.pageId;
    if (sourceId) {if(!entry.creation)await flushNoteReview(sourceId);holdTextEditor(sourceId,commit.id,true);}
    try {
      if (!entry.creation) {
        let source: NoteDraftCommit["source"];
        if (entry.source) {
          const before=await this.client.gateway.loadDocument(entry.source.pageId);
          const link: NoteBlockV1={id:noteBlockId(`page-link:${id}`),type:"researchObjectReference",props:{pageEntry:true},children:[],content:{kind:"reference",targetKind:"note_page",targetId:id,labelSnapshot:normalizeNoteTitle(commit.saved.title ?? ""),quoteSnapshot:""}};
          let found=!entry.source.blockId;
          const replace=(blocks:readonly NoteBlockV1[]):readonly NoteBlockV1[]=>blocks.map(block=>{
            if(block.id===entry.source!.blockId){
              if(entry.source!.anchor && !adapter.equal({...before.document,blocks:[entry.source!.anchor]},{...before.document,blocks:[block]})) throw new Error("MODEL_DRAFT_SOURCE_CHANGED：来源块已有新输入，新建操作未覆盖这些内容");
              found=true;return {...link,id:block.id,children:block.children};
            }
            return {...block,children:replace(block.children)};
          });
          const blocks=entry.source.blockId ? replace(before.document.blocks) : [...before.document.blocks,link];
          if(!found)throw new Error("MODEL_DRAFT_SOURCE_CHANGED：来源位置已删除，新页面未建立，已有输入保留");
          source={document:{...before.document,blocks},expectedRevision:before.page.revision,saveReason:"structure"};
          prepared=prepareNavigationSessions(before,source.document);
          if(prepared){preparation={port:prepared,dispatched:false};this.preparations.set(id,preparation);}
        }
        entry.creation={commit,favorite:entry.initial.page.isFavorite,...(source ? {source} : {})};
        await this.preserve(id,entry);
      }
      if (entry.creation.commit.id !== commit.id) throw new Error("MODEL_REQUEST_UNSETTLED：请先重试原新建页面请求");
      if(!dispatched){prepared?.dispatch();dispatched=true;if(preparation)preparation.dispatched=true;}
      const reply=await publishNoteDraft(id,entry.creation,async receipt=>{
        entry.materialized=receipt.page;entry.authority=receipt.snapshot;metadata(receipt.page.page);
        markNoteDraftIdentity(id,false);
        await this.preserve(id,entry);
      });
      // Preserve any typing after the immutable first request before removing
      // the provisional recovery slot. The same editor and history survive.
      try {
        if(reply.source)await prepared?.commit(reply.source);
        this.preparations.delete(id);
        if(entry.checkpoint)await getHbsDocumentGateway().savePreservation("note",id,entry.checkpoint);
        clearRecoveryDraft(identity(id));await flushStorage(getAppStorage());
      } catch(reason) {notify.error(`MODEL_DRAFT_RECOVERY_PENDING：笔记已建立，编辑保全或来源同步仍需核对：${String(reason)}（请求 ${commit.id}）`);}
      return reply;
    } catch(reason) {
      const code=reason && typeof reason === "object" && "code" in reason ? String(reason.code) : "";
      if(!dispatched || ["MODEL_INVALID_REQUEST","MODEL_INVALID_DOCUMENT","MODEL_SOURCE_SAVE_FAILED","revision_conflict","invalid_input","not_found"].includes(code)) {
        prepared?.cancel();this.preparations.delete(id);delete entry.creation;await this.preserve(id,entry);
      }
      else prepared?.recover();
      throw reason;
    } finally {if(sourceId)holdTextEditor(sourceId,commit.id,false);}
  }
}

export function reportNoteCreationError(reason:unknown):void{
  const retry=reason&&typeof reason==="object"&&"retryCreation"in reason?reason.retryCreation:undefined;
  notify.error(reason instanceof Error?reason.message:String(reason),typeof retry==="function"?{actions:[{id:"retry-note-creation",label:"重试创建",onClick:async()=>{
    try {
      const created=await retry() as NoteDocumentSnapshot;
      window.dispatchEvent(new CustomEvent("research-assistant:open-note-page",{detail:created.page.id}));
    } catch(error) {reportNoteCreationError(error);}
  }}]}:undefined);
}
