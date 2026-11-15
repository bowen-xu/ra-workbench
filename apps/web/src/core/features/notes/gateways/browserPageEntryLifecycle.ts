import {pageEntryTargets, type NoteDocumentV1, type NoteBlockV1, type NotePageId} from "@research-assistant/core/notes";
import {NotesGatewayError} from "@research-assistant/data/notes";
import type {BrowserNotesState} from "./browserNotesStorage";
import type {NoteDocumentSaveRequest} from "@research-assistant/data/notes";
export type PageEntryLifecycleMode="document"|"prepared";
export type BrowserDocumentSave=(state:BrowserNotesState,request:NoteDocumentSaveRequest,lifecycle?:PageEntryLifecycleMode)=>BrowserNotesState;
function fail(message:string):never{throw new NotesGatewayError("invalid_input",message);}
export function changedPageEntryTargets(before:NoteDocumentV1,after:NoteDocumentV1):ReadonlySet<string>{
  const a=pageEntryTargets(before),b=pageEntryTargets(after);
  return new Set([...a,...b].filter(id=>a.has(id)!==b.has(id)));
}
/** Sources linked to affected entrances only; no unrelated note bodies. */
export function pageEntryReadPages(id:string,targets:ReadonlySet<string>){
  return(metadata:BrowserNotesState):readonly string[]=>[...new Set([id,...targets,...metadata.links.flatMap(link=>
    link.target.kind==="note_page"&&targets.has(link.target.id)?[link.sourcePageId]:[])])];
}
function legacy(blocks:readonly NoteBlockV1[],target:string):NoteBlockV1|undefined{
  for(const block of blocks){if(block.type==="childPage"&&block.content.kind==="reference"&&block.content.targetId===target)return block;const found=legacy(block.children,target);if(found)return found;}
}
/** Lifecycle is part of source S/B/receipt's existing IDB transaction. The same
 * source history inverse therefore restores the page without another undo stack. */
export function reconcileBrowserPageEntries(state:BrowserNotesState,before:NoteDocumentV1,after:NoteDocumentV1,timestamp:string):BrowserNotesState{
  const old=pageEntryTargets(before),next=pageEntryTargets(after);
  if(next.has(after.pageId))fail("页面入口不能指向自身");
  const updates=new Map<string,"active"|"trashed">(),parents=new Map<string,NotePageId|undefined>();
  const graph=(id:string,depth=0,seen=new Set<string>()):string[]=>{
    if(id===after.pageId)fail("页面入口不能形成生命周期环路");
    if(depth>64||seen.size>20_000)fail("页面入口结构超过安全限制");
    if(seen.has(id))return[];seen.add(id);
    if(!state.pages.some(p=>p.id===id))return[];
    const doc=state.documents[id];if(!doc)fail("页面入口的关联正文尚未完整读取");
    return[id,...[...pageEntryTargets(doc)].flatMap(child=>graph(child,depth+1,seen))];
  };
  const held=(id:string)=>Object.values(state.documents).some(doc=>doc.pageId!==after.pageId
    &&state.pages.some(p=>p.id===doc.pageId&&(updates.get(p.id)??p.status)==="active")&&pageEntryTargets(doc).has(id));
  for(const id of next)if(!old.has(id)){
    const page=state.pages.find(p=>p.id===id);if(!page)fail("页面入口目标不存在");
    const tree=graph(id),entry=legacy(after.blocks,id);
    if(entry){
      const proven=state.revisions.some(rev=>rev.pageId===after.pageId&&legacy(rev.document.blocks,id)?.id===entry.id);
      if(proven&&page.parentPageId!==after.pageId)parents.set(id,after.pageId);
    }else if(page.status==="trashed")parents.set(id,undefined);
    if(page.status==="trashed")for(const target of tree)if(state.pages.find(p=>p.id===target)?.status==="trashed")updates.set(target,"active");
  }
  for(const id of old)if(!next.has(id)){
    if(held(id)){if(state.pages.find(p=>p.id===id)?.parentPageId===after.pageId)parents.set(id,undefined);continue;}
    for(const child of graph(id))if(!next.has(child)&&!held(child))updates.set(child,"trashed");
  }
  if(!updates.size&&!parents.size)return state;
  return{...state,pages:state.pages.map(page=>{
    const status=updates.get(page.id)??page.status,parent=parents.has(page.id)?parents.get(page.id):page.parentPageId;
    if(status===page.status&&parent===page.parentPageId)return page;
    const{trashedAt:_trashed,parentPageId:_parent,...base}=page;
    return{...base,status,revision:page.revision+1,updatedAt:timestamp,...(parent?{parentPageId:parent}:{}),...(status==="trashed"?{trashedAt:timestamp}:{})};
  })};
}
