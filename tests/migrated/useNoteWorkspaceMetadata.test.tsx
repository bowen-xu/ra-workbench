// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {beforeEach,afterEach,expect,it,vi} from "vitest";
import {notePageId,type NotePage} from "@research-assistant/core/notes";
import {NotesGatewayError,type NoteDocumentSnapshot,type NotesGateway} from "@research-assistant/data/notes";
import {NotesClient} from "../../apps/web/src/core/features/notes/NotesClient";
import {useNoteDocumentMetadata as useNoteWorkspaceMetadata} from "../../apps/web/src/core/public/noteDocuments";

const observe=vi.fn(),onError=vi.fn();
let root:ReturnType<typeof createRoot>,host:HTMLDivElement,client:NotesClient,metadata:ReturnType<typeof vi.fn>,reading:ReturnType<typeof vi.fn>,catalog:ReturnType<typeof vi.fn>;
let output:ReturnType<typeof useNoteWorkspaceMetadata>;
function snapshot(id:string,parent?:string):NoteDocumentSnapshot{
  const page:NotePage={id:notePageId(id),title:id,revision:1,position:0,status:"active",schemaVersion:1,isFavorite:false,projectIds:[],tagIds:[],createdAt:"2026-10-24T00:00:00Z",updatedAt:"2026-10-24T00:00:00Z",...(parent?{parentPageId:notePageId(parent)}:{})};
  return{page,document:{schemaVersion:1,pageId:page.id,blocks:[]}};
}
const opened=[snapshot("open")];
function Harness({active=true,values=opened,version=client.getVersion()}:{active?:boolean;values?:readonly NoteDocumentSnapshot[];version?:number}){
  output=useNoteWorkspaceMetadata({client,version,active,snapshots:values,current:values[0],source:undefined,observe,onError});return<div>{output.pages.map(page=>page.title).join(",")}</div>;
}
beforeEach(()=>{
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);observe.mockReset();onError.mockReset();
  metadata=vi.fn(async(ids:readonly string[])=>ids.map(id=>snapshot(id).page));reading=vi.fn(async()=>{throw new NotesGatewayError("not_found","missing source");});catalog=vi.fn();
  client=new NotesClient({listPageMetadata:metadata,getReadingNote:reading,pageCatalog:{queryNotePageCatalog:catalog}}as unknown as NotesGateway);
  host=document.createElement("div");document.body.append(host);root=createRoot(host);
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();});

it("does not enumerate metadata or load any reading body when the list has no open editors",async()=>{
  await act(async()=>root.render(<Harness values={[]}/>));expect(metadata).not.toHaveBeenCalled();expect(reading).not.toHaveBeenCalled();expect(catalog).not.toHaveBeenCalled();
});
it("reuses opened authority and does not reread it on five unchanged hot trips",async()=>{
  await act(async()=>root.render(<Harness/>));expect(reading).toHaveBeenCalledOnce();expect(metadata).not.toHaveBeenCalled();
  for(let index=0;index<5;index++){await act(async()=>root.render(<Harness active={false}/>));await act(async()=>root.render(<Harness/>));}
  expect(reading).toHaveBeenCalledOnce();expect(metadata).not.toHaveBeenCalled();expect(host.textContent).toBe("open");
});
it("only reads the required breadcrumb chain and retains its metadata",async()=>{
  metadata.mockImplementation(async(ids:readonly string[])=>ids.map(id=>snapshot(id,id==="parent"?"root":undefined).page));
  const values=[snapshot("open","parent")];await act(async()=>root.render(<Harness values={values}/>));
  expect(metadata.mock.calls).toEqual([[["parent"]],[["root"]]]);expect(host.textContent).toBe("open,parent,root");
  await act(async()=>root.render(<Harness values={values} active={false}/>));await act(async()=>root.render(<Harness values={values}/>));expect(metadata).toHaveBeenCalledTimes(2);
});
it("ignores unrelated document and tag changes, without warming an index or fetching another body",async()=>{
  await act(async()=>root.render(<Harness/>));client.notifyChanged({kind:"document",pageId:notePageId("elsewhere"),revision:4});client.notifyChanged({kind:"tags"});
  await act(async()=>root.render(<Harness/>));expect(reading).toHaveBeenCalledOnce();expect(metadata).not.toHaveBeenCalled();expect(observe).not.toHaveBeenCalled();
});
it("observes only the externally changed opened identity and keeps retained editors outside the current list",async()=>{
  const values=[...opened,snapshot("retained")];await act(async()=>root.render(<Harness values={values}/>));
  client.notifyChanged({kind:"document",pageId:notePageId("retained"),revision:4});metadata.mockResolvedValueOnce([{...snapshot("retained").page,revision:4}]);
  await act(async()=>root.render(<Harness values={values}/>));expect(metadata.mock.calls).toEqual([[["retained"]]]);expect(observe).toHaveBeenCalledWith([expect.objectContaining({id:"retained",revision:4})]);expect(reading).toHaveBeenCalledOnce();
});
it("defers all-resource validation while hidden and reads only opened identities after resuming",async()=>{
  await act(async()=>root.render(<Harness/>));await act(async()=>root.render(<Harness active={false}/>));client.notifyChanged();
  await act(async()=>root.render(<Harness active={false}/>));expect(metadata).not.toHaveBeenCalled();
  await act(async()=>root.render(<Harness/>));expect(metadata.mock.calls).toEqual([[["open"]]]);expect(reading).toHaveBeenCalledTimes(2);
});
it("does not lose an invalidation when an already sent metadata request is superseded",async()=>{
  await act(async()=>root.render(<Harness/>));client.notifyChanged();let release!:(pages:readonly NotePage[])=>void;
  metadata.mockReturnValueOnce(new Promise(resolve=>{release=resolve;}));await act(async()=>root.render(<Harness/>));
  await act(async()=>root.render(<Harness active={false}/>));release([{...opened[0]!.page,revision:99}]);await act(async()=>Promise.resolve());
  expect(observe).not.toHaveBeenCalled();await act(async()=>root.render(<Harness/>));expect(metadata).toHaveBeenCalledTimes(2);expect(observe).toHaveBeenCalledOnce();
});
it("preserves the source failure details and permits a later retry on activation",async()=>{
  reading.mockRejectedValueOnce(new NotesGatewayError("source_unavailable","source-read-failed: operation=fixture-8"));
  await act(async()=>root.render(<Harness/>));expect(onError).toHaveBeenCalledWith(expect.stringContaining("fixture-8"));
  await act(async()=>root.render(<Harness active={false}/>));await act(async()=>root.render(<Harness/>));expect(reading).toHaveBeenCalledTimes(2);
});
