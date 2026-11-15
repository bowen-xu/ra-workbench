import {decodeNoteDocument, notePageId, type NoteDocumentV1} from "@research-assistant/core/notes";
import type {NotesGateway} from "@research-assistant/data/notes";
import {getAppStorage} from "../../../runtime/appStorage";
import {getAppRuntime} from "../../../runtime/appRuntime";
import {getHbsDocumentGateway,readHbsDocumentDiagnostics} from "../../../services/hbsDocumentGateway";
import {recoveryDraftKey} from "../../../services/recoveryDraftStore";
import {nativeCommandDiagnostics} from "../../../runtime/NativeCommandDiagnostics";
import {inspectNoteHbsSession, noteEditorDocument, noteReviewRecovery} from "../editor/noteTextEditorPorts";
import {collectNoteAssets} from "./noteMarkdown";
import {diagnosticError, documentFingerprint, sessionDiagnostics} from "./noteRecoveryDiagnostics";
import {version} from "../../../../../package.json";
import {createNoteDocumentAdapter} from "@research-assistant/core/hbs";
import {createDocumentPort,createDocumentPreservationPort} from "@research-assistant/data/hbs";
import {directTextDiff} from "../../../components/hbs/codeMirrorHbsPolicy";
import {storagePersistence,storageBackend} from "../../../runtime/storagePersistence";

type Material = {readonly source:string;readonly value?:unknown;readonly error?:Record<string,unknown>};
export interface NoteRecoveryCollection {
  readonly schemaVersion:1;
  readonly kind:"research-assistant-note-recovery";
  readonly exportedAt:string;
  readonly pageId:string;
  readonly materials:readonly Material[];
  readonly assets:readonly {id:string;mimeType:string;byteSize?:number;base64?:string;error?:Record<string,unknown>}[];
  readonly missing:readonly string[];
  readonly diagnostics:Record<string,unknown>;
}

/** A best-effort, read-only capture. Independent sources survive another source's
 * failure. No flush, replay, migration, reference-count mutation or cleanup. */
export async function collectNoteRecovery(pageId:string, notes:NotesGateway, status:{status:string;message?:string}, includeAssets=true):Promise<NoteRecoveryCollection> {
  const materials:Material[]=[], missing:string[]=[];
  async function capture(source:string, read:()=>unknown|Promise<unknown>) {
    try {const value=await read();materials.push({source,value:value===undefined?null:structuredClone(value)});return value;}
    catch(reason){const error=diagnosticError(reason);materials.push({source,error});missing.push(source);return undefined;}
  }
  // Freeze memory before awaiting storage, so later input cannot replace it.
  const session=inspectNoteHbsSession(pageId);
  await capture("session",()=>session);
  await capture("editor",()=>noteEditorDocument(pageId));
  await capture("metadataRecovery",()=>noteReviewRecovery(pageId));
  const storage=getAppStorage();
  const slots=[
    ["outbox",`research-assistant:hbs-outbox:note:${pageId}`],
    ["legacyConflictRequest",`research-assistant.note-conflict.${pageId}`],
    ["legacyDraft",recoveryDraftKey({scope:"note-page",resourceId:pageId,slot:"document"})],
  ] as const;
  for(const [source,key] of slots) await capture(source,()=>{
    const raw=storage.getItem(key);if(raw===null)return null;
    try{return JSON.parse(raw) as unknown;}catch{return {unparsed:raw};}
  });
  const hbs=getHbsDocumentGateway();
  const authority=await capture("authority",()=>hbs.read("note",pageId));
  const checkpoint=await capture("checkpoint",()=>hbs.readPreservation("note",pageId));
  const adapter=createNoteDocumentAdapter(directTextDiff);
  if(authority!==undefined)await capture("authorityValidation",async()=>{await createDocumentPort(adapter,async()=>authority).read();return {valid:true};});
  if(checkpoint!==undefined)await capture("checkpointValidation",async()=>{await createDocumentPreservationPort(adapter,async()=>checkpoint).load();return {valid:true};});
  await capture("page",()=>notes.loadDocument(notePageId(pageId)));
  await capture("outgoingLinks",()=>notes.listOutgoing(notePageId(pageId)));
  await capture("backlinks",()=>notes.listBacklinks("note_page",pageId));
  if(includeAssets){
    const revisions=await capture("revisionIndex",()=>notes.listRevisions(notePageId(pageId)));
    if(Array.isArray(revisions))for(const revision of revisions)await capture(`revision:${revision.id}`,()=>notes.loadRevision(notePageId(pageId),revision.id));
  }
  const documents:NoteDocumentV1[]=[];
  function walk(value:unknown):void {
    if(!value||typeof value!=="object")return;
    if("schemaVersion" in value&&"pageId" in value&&"blocks" in value){
      try {const doc=decodeNoteDocument(value);if(doc.pageId===pageId)documents.push(doc);}catch{/* Raw damaged material remains in the recovery package. */}
      return;
    }
    for(const item of Object.values(value))walk(item);
  }
  for(const material of materials)walk(material.value);
  if(!documents.length)missing.push("readableDocument");
  const references=new Map(documents.flatMap(collectNoteAssets).map(asset=>[asset.id,asset]));
  const assets:NoteRecoveryCollection["assets"][number][]=[];
  if(includeAssets)for(const asset of references.values()){
    try {const bytes=await notes.readNoteAsset(asset.id);let binary="";
      for(let i=0;i<bytes.length;i+=32768)binary+=String.fromCharCode(...bytes.subarray(i,i+32768));
      assets.push({id:asset.id,mimeType:asset.mimeType,byteSize:bytes.length,base64:btoa(binary)});
    }catch(reason){missing.push(`asset:${asset.id}`);assets.push({id:asset.id,mimeType:asset.mimeType,error:diagnosticError(reason)});}
  }
  const fingerprints=await Promise.all(materials.map(async item=>({source:item.source,
    ...(item.error?{error:item.error}:await documentFingerprint(item.value))})));
  const pending=materials.find(item=>item.source==="outbox")?.value;
  const request=pending&&typeof pending==="object"?pending as Record<string,unknown>:{};
  const persistence=storagePersistence(storage);
  const diagnostics={schemaVersion:1,kind:"research-assistant-note-diagnostics",exportedAt:new Date().toISOString(),pageId,
    app:{version,mode:getAppRuntime().mode,userAgent:navigator.userAgent},
    status:{status:status.status,error:diagnosticError(status.message??"")},
    session:sessionDiagnostics(session),
    request:{id:typeof request.id==="string"&&/^[\w:.-]{1,200}$/.test(request.id)?request.id:undefined,expectedVersion:typeof request.expectedVersion==="number"?request.expectedVersion:undefined},
    storage:{backend:storageBackend(storage),pending:persistence?.pending,resetting:persistence?.resetting,error:persistence?.error?diagnosticError(persistence.error):undefined},
    sources:fingerprints,referencedAssetCount:references.size,
    hbsOperations:readHbsDocumentDiagnostics("note",pageId),
    missing:[...missing],nativeCommands:nativeCommandDiagnostics?.read()??{available:false},
    privacy:"No document text, titles, asset bytes, raw requests or provider payloads. Event sequence has no timestamps."};
  return {schemaVersion:1,kind:"research-assistant-note-recovery",exportedAt:new Date().toISOString(),pageId,materials,assets,missing,diagnostics};
}
