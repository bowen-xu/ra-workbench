import {useCallback,useEffect,useRef,useState} from "react";
import {noteBlockId,type NotePage,type NotePageId,type ReadingNoteBlock} from "@research-assistant/core/notes";
import {NotesGatewayError} from "@research-assistant/data/notes";
import type {NoteResourceAccess} from "@research-assistant/workbench-sdk/note-resources";
import type {NoteDocumentMetadata, NoteDocumentMetadataOptions} from "@research-assistant/workbench-sdk/note-metadata";
import {catalogErrorMessage} from "../noteCatalogError";

/** Editor ownership is independent of the list's current page. Observe only
 * opened identities and breadcrumb ancestors; partial list results must never
 * become an authoritative inventory of existing/editable documents. */
export function useNoteDocumentMetadata(options: NoteDocumentMetadataOptions): NoteDocumentMetadata {
  const {client,version,active,current,source,observe,onError}=options;
  const live=useRef(options);live.current=options;
  const [pages,setPages]=useState<readonly NotePage[]>([]),[readingNotes,setReadingNotes]=useState<readonly ReadingNoteBlock[]>([]);
  const known=useRef(new Map<NotePageId,NotePage>()),observed=useRef({client,version});
  const sources=useRef(new Map<NotePageId,{readonly client:NoteResourceAccess;readonly version:number;readonly revision?:number}>());
  const rememberReadingNote=useCallback((note:ReadingNoteBlock)=>{
    if(!note.pageId)return;
    sources.current.delete(note.pageId);sources.current.set(note.pageId,{client,version,revision:note.revision});
    if(sources.current.size>128)sources.current.delete(sources.current.keys().next().value!);
    setReadingNotes(value=>value[0]===note?value:[note,...value.filter(previous=>previous.id!==note.id)].slice(0,128));
  },[client,version]);
  const signature=options.snapshots.map(value=>`${value.page.id}:${value.page.revision}`).join("\0");
  useEffect(()=>{
    let valid=true;
    if(observed.current.client!==client){known.current.clear();sources.current.clear();}
    const publish=(values:readonly NotePage[])=>{
      if(!valid)return;
      for(const page of values)if(page.revision>=(known.current.get(page.id)?.revision??-1)){known.current.delete(page.id);known.current.set(page.id,page);}
      const retained=new Set(live.current.snapshots.map(value=>value.page.id));
      for(const id of known.current.keys())if(known.current.size>128&&!retained.has(id))known.current.delete(id);
      setPages([...known.current.values()]);
    };
    publish(live.current.snapshots.map(value=>value.page));
    if(!active)return()=>{valid=false;};
    const previous=observed.current;
    void(async()=>{
      const snapshots=live.current.snapshots,changes=previous.client===client?client.getChangesSince(previous.version):[{kind:"all" as const}];
      const retained=new Map(snapshots.map(value=>[value.page.id,value.page]));
      const all=changes.some(change=>change.kind==="all");
      const ids=all?[...retained.keys()]:[...new Set(changes.flatMap(change=>change.kind==="document"&&retained.has(change.pageId)&&(retained.get(change.pageId)!.revision<change.revision)?[change.pageId]:change.kind==="metadata"?change.receipt.pages.filter(page=>retained.has(page.id)&&retained.get(page.id)!.revision<page.revision).map(page=>page.id):[]))];
      for(let offset=0;valid&&offset<ids.length;offset+=200){const result=await client.gateway.listPageMetadata(ids.slice(offset,offset+200));if(!valid)return;publish(result);observe(result);}
      let parent=live.current.current?.page.parentPageId;
      const visited=new Set<NotePageId>();
      while(valid&&parent&&!visited.has(parent)){
        visited.add(parent);let page=known.current.get(parent);
        if(!page){const result=await client.gateway.listPageMetadata([parent]);if(!valid)return;publish(result);page=result.find(value=>value.id===parent);}
        parent=page?.parentPageId;
      }
      if(valid)observed.current={client,version};
    })().catch(reason=>{if(valid)onError(catalogErrorMessage(reason));});
    return()=>{valid=false;};
  },[client,version,active,signature,current?.page.parentPageId,observe,onError]);
  useEffect(()=>{
    if(!active||!current)return;
    if(source?.pageId===current.page.id){rememberReadingNote(source);return;}
    const cached=sources.current.get(current.page.id);
    if(cached?.client===client&&(cached.revision===undefined||cached.revision>=current.page.revision)&&client.getChangesSince(cached.version).every(change=>change.kind==="tags"||change.kind==="document"&&change.pageId!==current.page.id||change.kind==="metadata"&&!change.receipt.pages.some(page=>page.id===current.page.id))){
      sources.current.set(current.page.id,{...cached,version});return;
    }
    let valid=true;
    const remember=()=>{sources.current.delete(current.page.id);sources.current.set(current.page.id,{client,version,revision:current.page.revision});if(sources.current.size>128)sources.current.delete(sources.current.keys().next().value!);};
    void client.gateway.getReadingNote(noteBlockId(current.page.id)).then(note=>{if(valid&&note.pageId===current.page.id)rememberReadingNote(note);})
      .catch(reason=>{if(!valid)return;if(reason instanceof NotesGatewayError&&reason.code==="not_found"){remember();setReadingNotes(value=>value.some(note=>note.pageId===current.page.id)?value.filter(note=>note.pageId!==current.page.id):value);return;}onError(catalogErrorMessage(reason));});
    return()=>{valid=false;};
  },[client,active,current?.page.id,version,source,onError,rememberReadingNote]);
  return{pages,readingNotes,rememberReadingNote};
}
