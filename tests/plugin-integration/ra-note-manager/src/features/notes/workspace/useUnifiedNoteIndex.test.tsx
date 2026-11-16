// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {beforeEach,afterEach,expect,it,vi} from "vitest";
import {noteBlockId,notePageId,queryNoteIndex,type NoteIndexRecord,type NoteCategory,type NoteIndexView} from "@research-assistant/core/notes";
import {NotesGatewayError,type NoteDocumentSnapshot,type NotesGateway,type NotePageCatalogQuery,type NotePageCatalogPage} from "@research-assistant/data/notes";
import {NotesClient} from "../../../../../../../apps/web/src/core/features/notes/NotesClient";
import {useUnifiedNoteIndex} from "../../../../../../../../plugins/ra-note-manager/src/features/notes/workspace/useUnifiedNoteIndex";

const f=vi.hoisted(()=>({writing:vi.fn(),prepare:vi.fn(),available:true}));
vi.mock("../../../../../../../apps/web/src/core/public/domainCapabilities",()=>({getOptionalWritingNoteCatalog:()=>f.available?{queryWritingNoteCatalog:f.writing,prepareWritingNoteCatalog:f.prepare}:undefined}));
let root:ReturnType<typeof createRoot>,host:HTMLDivElement,output:ReturnType<typeof useUnifiedNoteIndex>,client:NotesClient;
let records:NoteIndexRecord[],queryPages:ReturnType<typeof vi.fn<(query:NotePageCatalogQuery)=>Promise<NotePageCatalogPage>>>,all:ReturnType<typeof vi.fn>,reorder:ReturnType<typeof vi.fn>;
function result(query:NotePageCatalogQuery):NotePageCatalogPage{
  // Adapter stub only; production uses SQLite/IndexedDB. The UI is not supplied
  // a corpus or a body, only this bounded response.
  return{...queryNoteIndex(records,{...query,search:query.search??""}),revision:1};
}
function Harness({query="",enabled=true,version=0,library=false,explorer=true,category="all",view="recent"}:{query?:string;enabled?:boolean;version?:number;library?:boolean;explorer?:boolean;category?:NoteCategory;view?:NoteIndexView}){
  output=useUnifiedNoteIndex({client,changeVersion:version,enabled,category,view,query,libraryEnabled:library,explorerEnabled:explorer});
  return<div>{output.items.map(item=>item.title).join(" ")}</div>;
}
beforeEach(()=>{
  f.available=true;vi.useFakeTimers();vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  records=Array.from({length:100},(_,index)=>({id:`page:${index}`,title:`Note ${String(100-index).padStart(3,"0")}`,snippet:"",category:"independent",status:"active",isFavorite:false,searchText:`saved needle ${index}`,updatedAt:new Date(Date.UTC(2026,9,24)-index*1000).toISOString(),target:{kind:"page",pageId:notePageId(String(index))}}));
  queryPages=vi.fn(async query=>result(query));all=vi.fn(()=>{throw Error("unexpected corpus/body enumeration");});reorder=vi.fn(async()=>{});
  client=new NotesClient({pageCatalog:{queryNotePageCatalog:queryPages},listPages:all,listReadingNotes:all,loadIndexDocuments:all,loadDocument:all,
    listFavoritePageIds:async()=>records.map(item=>(item.target as {pageId:ReturnType<typeof notePageId>}).pageId),reorderFavorites:reorder}as unknown as NotesGateway);
  f.writing.mockReset().mockResolvedValue({items:[],revision:1,pendingCount:0,errorCount:0,issues:[]});f.prepare.mockReset().mockResolvedValue({processedCount:0,pendingCount:0,errorCount:0});
  host=document.createElement("div");document.body.append(host);root=createRoot(host);
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.useRealTimers();vi.unstubAllGlobals();});

it("renders only the requested twenty and never requests full metadata or canonical bodies",async()=>{
  await act(async()=>root.render(<Harness/>));expect(output.items).toHaveLength(20);expect(output.hasMore).toBe(true);
  expect(queryPages.mock.calls.map(([q])=>q.limit)).toEqual([20,1]);expect(f.writing.mock.calls.map(([q])=>q.limit)).toEqual([20,1]);
  expect(all).not.toHaveBeenCalled();expect(f.prepare).not.toHaveBeenCalled();
});
it("retains successful pages across ten hot trips and defers invalidation while hidden",async()=>{
  await act(async()=>root.render(<Harness/>));
  for(let i=0;i<10;i++){await act(async()=>root.render(<Harness enabled={false}/>));await act(async()=>root.render(<Harness/>));}
  expect(queryPages).toHaveBeenCalledTimes(2);
  await act(async()=>root.render(<Harness enabled={false}/>));await act(async()=>window.dispatchEvent(new Event("research-assistant:writing-notes-changed")));
  expect(queryPages).toHaveBeenCalledTimes(2);await act(async()=>root.render(<Harness/>));expect(queryPages).toHaveBeenCalledTimes(4);expect(all).not.toHaveBeenCalled();
});
it("drains one uncancellable cold read across repeated hidden trips without publishing it",async()=>{
  let release!:(value:NotePageCatalogPage)=>void;queryPages.mockReturnValueOnce(new Promise(resolve=>{release=resolve;}));
  await act(async()=>root.render(<Harness/>));
  for(let i=0;i<10;i++){await act(async()=>root.render(<Harness enabled={false}/>));await act(async()=>root.render(<Harness/>));}
  await act(async()=>root.render(<Harness enabled={false}/>));expect(queryPages).toHaveBeenCalledOnce();
  await act(async()=>release(result({category:"all",view:"recent",limit:20})));
  expect(output.items).toHaveLength(0);expect(queryPages).toHaveBeenCalledOnce();
  await act(async()=>root.render(<Harness/>));expect(queryPages).toHaveBeenCalledTimes(3);expect(output.items).toHaveLength(20);
});
it("debounces typed queries, never publishes old results and does not run an unused initial browse",async()=>{
  let release!:(value:NotePageCatalogPage)=>void;queryPages.mockReturnValueOnce(new Promise(resolve=>{release=resolve;}));
  await act(async()=>root.render(<Harness query="older"/>));expect(queryPages.mock.calls[0]![0].search).toBe("older");
  await act(async()=>root.render(<Harness query="needle"/>));await act(async()=>vi.advanceTimersByTime(220));expect(queryPages).toHaveBeenCalledOnce();
  await act(async()=>release(result({category:"all",view:"recent",search:"older",limit:20})));
  expect(output.items).toHaveLength(20);expect(queryPages.mock.calls.filter(([q])=>q.limit===20).map(([q])=>q.search)).toEqual(["older","needle"]);expect(output.error).toBeUndefined();
});
it("queries separate sidebar/library ordering and appends only the next metadata page",async()=>{
  await act(async()=>root.render(<Harness/>));const sidebar=output.items;
  await act(async()=>root.render(<Harness library explorer={false}/>));expect(output.libraryItems).toHaveLength(50);expect(output.libraryHasMore).toBe(true);
  await act(async()=>output.setLibrarySort({field:"title",direction:"ascending"}));expect(output.libraryItems[0]!.id).toBe("page:99");expect(output.items).toBe(sidebar);
  await act(async()=>output.loadLibraryMore());expect(output.libraryItems).toHaveLength(100);expect(output.libraryHasMore).toBe(false);expect(all).not.toHaveBeenCalled();
});
it("searches all live categories while preserving browse filters",async()=>{
  await act(async()=>root.render(<Harness query="needle" category="dependent" view="favorites"/>));expect(output.items).toHaveLength(20);
  expect(queryPages.mock.calls[0]![0]).toMatchObject({category:"dependent",view:"favorites",search:"needle",limit:20});
  await act(async()=>root.render(<Harness category="dependent" view="favorites"/>));await act(async()=>vi.advanceTimersByTime(220));expect(output.items).toHaveLength(0);
});
it("preserves a concrete source error and retries via the catalog instead of a body rebuild",async()=>{
  queryPages.mockRejectedValueOnce(new NotesGatewayError("database_busy","operation=fixture-7 HTTP 503"));
  await act(async()=>root.render(<Harness/>));expect(output.error).toContain("database_busy");expect(output.error).toContain("fixture-7");
  await act(async()=>output.retry());expect(output.error).toBeUndefined();expect(output.items).toHaveLength(20);expect(all).not.toHaveBeenCalled();
});
it("collects all matching metadata only after explicit select-all, with cancellation",async()=>{
  await act(async()=>root.render(<Harness library explorer={false}/>));const before=queryPages.mock.calls.length;expect(output.libraryItems).toHaveLength(50);
  const controller=new AbortController();let selected:readonly unknown[]=[];
  await act(async()=>{selected=await output.collectAll(controller.signal);});expect(selected).toHaveLength(100);expect(queryPages.mock.calls.slice(before).map(([q])=>q.limit)).toEqual([200,1]);
  controller.abort();await act(async()=>{await expect(output.collectAll(controller.signal)).rejects.toMatchObject({name:"AbortError"});});expect(all).not.toHaveBeenCalled();
});
it("reorders only requested favorites within the canonical expected order",async()=>{
  records=records.map((item,index)=>({...item,isFavorite:true,favoritePosition:index}));
  await act(async()=>root.render(<Harness view="favorites" category="independent"/>));
  await act(async()=>output.reorderFavorites(["page:1","page:0",...output.items.slice(2).map(item=>item.id)]));
  expect(reorder).toHaveBeenCalledWith({pageIds:[notePageId("1"),notePageId("0"),...Array.from({length:98},(_,i)=>notePageId(String(i+2)))],expectedPageIds:Array.from({length:100},(_,i)=>notePageId(String(i)))});expect(f.writing).not.toHaveBeenCalled();expect(all).not.toHaveBeenCalled();
});

it("shows a missing writing catalog explicitly, preserves Core records, and restores the optional source after reassembly",async()=>{
  f.available=false;const before=JSON.stringify(records);
  await act(async()=>root.render(<Harness/>));
  expect(output.items).toHaveLength(20);expect(f.writing).not.toHaveBeenCalled();
  expect(output.warnings.join(" ")).toContain("writing_note_catalog_unavailable");
  expect(JSON.stringify(records)).toBe(before);expect(all).not.toHaveBeenCalled();
  await act(async()=>root.unmount());f.available=true;root=createRoot(host);
  await act(async()=>root.render(<Harness/>));
  expect(f.writing).toHaveBeenCalled();expect(output.warnings.join(" ")).not.toContain("writing_note_catalog_unavailable");
  expect(JSON.stringify(records)).toBe(before);
});

it("restores a nonempty writing resource by its original identity without copying or rewriting Core notes",async()=>{
  const coreId=notePageId("core-with-writing-reference"),manuscriptId="existing-manuscript",writingPath="notes/existing-idea.ranote";
  const sourceHref=`source://writings/${manuscriptId}/${writingPath}`;
  const canonical:NoteDocumentSnapshot={
    page:{id:coreId,title:"Core canonical note",position:0,status:"active",isFavorite:false,revision:7,schemaVersion:1,
      projectIds:[],tagIds:[],createdAt:"2026-10-24T00:00:00Z",updatedAt:"2026-10-24T00:00:00Z"},
    document:{schemaVersion:1,pageId:coreId,blocks:[{id:noteBlockId("writing-reference"),type:"paragraph",props:{},children:[],
      content:{kind:"inline",items:[{type:"text",text:"Core body stays canonical. ",styles:[]},
        {type:"link",href:sourceHref,content:[{type:"text",text:"Original writing note",styles:[]}]}]}}]},
  };
  records=[{id:`page:${coreId}`,title:canonical.page.title,snippet:"Core body stays canonical.",searchText:"Core canonical note",
    category:"independent",status:"active",isFavorite:false,updatedAt:canonical.page.updatedAt,target:{kind:"page",pageId:coreId}}];
  const writingRecord:NoteIndexRecord={id:`writing:${JSON.stringify([manuscriptId,"original-writing-note-id"])}`,
    title:"Original writing note",snippet:"Existing writing content",searchText:"Original writing note Existing writing content",
    category:"dependent",owner:{kind:"manuscript",id:manuscriptId,label:"Original manuscript"},status:"active",isFavorite:false,
    updatedAt:"2026-10-24T01:00:00Z",target:{kind:"writing",manuscriptId,path:writingPath}};
  f.writing.mockImplementation(async(query:NotePageCatalogQuery)=>({...queryNoteIndex([writingRecord],{...query,search:query.search??""}),
    revision:9,pendingCount:0,errorCount:0,issues:[]}));
  const loadDocument=vi.fn<NotesGateway["loadDocument"]>(async id=>{expect(id).toBe(coreId);return canonical;});
  const unsupportedWrite=():never=>{throw Error("provider recovery must not create or rewrite Core documents");};
  const createPage=vi.fn<NotesGateway["createPage"]>(unsupportedWrite),saveDocument=vi.fn<NotesGateway["saveDocument"]>(unsupportedWrite);
  Object.assign(client.gateway,{loadDocument,createPage,saveDocument});
  const owner=client,history=client.history,mutate=vi.spyOn(client,"mutate");
  const before=structuredClone(await client.gateway.loadDocument(coreId));
  const reassemble=async()=>{await act(async()=>root.unmount());root=createRoot(host);await act(async()=>root.render(<Harness/>));};

  await act(async()=>root.render(<Harness/>));
  const original=output.items.find(item=>item.id===writingRecord.id);
  expect(original).toMatchObject({id:writingRecord.id,target:writingRecord.target,owner:writingRecord.owner});
  expect(output.items.map(item=>item.id)).toContain(`page:${coreId}`);

  f.available=false;f.writing.mockClear();await reassemble();
  expect(output.items.map(item=>item.id)).toEqual([`page:${coreId}`]);expect(output.error).toBeUndefined();
  expect(output.warnings.join(" ")).toContain("writing_note_catalog_unavailable");expect(f.writing).not.toHaveBeenCalled();
  expect(await client.gateway.loadDocument(coreId)).toEqual(before);

  f.available=true;await reassemble();
  expect(output.items.find(item=>item.id===writingRecord.id)).toEqual(original);
  expect(output.items.map(item=>item.id)).toEqual([writingRecord.id,`page:${coreId}`]);
  expect(output.warnings.join(" ")).not.toContain("writing_note_catalog_unavailable");expect(f.writing).toHaveBeenCalled();
  expect(client).toBe(owner);expect(client.history).toBe(history);
  expect(await client.gateway.loadDocument(coreId)).toEqual(before);
  expect(canonical.document.blocks[0]?.content).toMatchObject({kind:"inline",items:expect.arrayContaining([
    expect.objectContaining({type:"link",href:sourceHref}),
  ])});
  expect(loadDocument).toHaveBeenCalledTimes(3);expect(records.map(item=>item.id)).toEqual([`page:${coreId}`]);
  expect(mutate).not.toHaveBeenCalled();expect(createPage).not.toHaveBeenCalled();expect(saveDocument).not.toHaveBeenCalled();
  expect(f.prepare).not.toHaveBeenCalled();expect(all).not.toHaveBeenCalled();mutate.mockRestore();
});
