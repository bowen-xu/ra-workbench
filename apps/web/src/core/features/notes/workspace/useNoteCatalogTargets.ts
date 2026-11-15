import {useMemo} from "react";
import {NotesGatewayError,queryNoteTargetCatalog,type NoteTargetQuery,type NoteTargetCatalogPage} from "@research-assistant/data/notes";
import type {NotesClient} from "../NotesClient";

/** No request on mounting/activation. Only an opened picker or mention query
 * reads the catalog; metadata pages are bounded and coalesce by data version. */
export function useNoteCatalogTargets(client:NotesClient,version:number):NoteTargetQuery{
  return useMemo(()=>{
    const cache=new Map<string,Promise<NoteTargetCatalogPage>>();
    return(search:string,cursor?:string)=>{
      const key=JSON.stringify([search,cursor]);const existing=cache.get(key);if(existing)return existing;
      const promise=Promise.resolve().then(()=>{
        if(!client.gateway.pageCatalog)throw new NotesGatewayError("source_unavailable","note_catalog_adapter_unavailable：无法读取笔记候选目录。");
        return queryNoteTargetCatalog(client.gateway.pageCatalog,search,cursor);
      });
      cache.set(key,promise);if(cache.size>32)cache.delete(cache.keys().next().value!);
      void promise.catch(()=>{if(cache.get(key)===promise)cache.delete(key);});return promise;
    };
  },[client,version]);
}
