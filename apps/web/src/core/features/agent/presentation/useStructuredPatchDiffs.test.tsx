/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach,expect,it,vi} from "vitest";
import {useStructuredPatchDiffs} from "./useStructuredPatchDiffs";
import {toolPatchDiffFiles} from "./toolPatchDiffModel";
import {toolActivityRows,type ToolActivityRow} from "./toolActivityModel";
import {countWritingChangedLines} from "./writingChangeStats";
const gateway=vi.hoisted(()=>({getVersion:vi.fn(),get:vi.fn()}));
const resolveGateway=vi.hoisted(()=>vi.fn());
vi.mock("../../../services/structuredResourceGateway",()=>({getStructuredResourceGateway:()=>{resolveGateway();return gateway;}}));
vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
const cleanups:(()=>Promise<void>)[]=[];
afterEach(async()=>{for(const cleanup of cleanups.splice(0))await cleanup();vi.resetAllMocks();});
function row(native=false){return toolActivityRows([{type:"tool-call",name:"research_apply_patch",callId:"a",content:JSON.stringify({patch:"*** Begin Patch\n*** Update File: portrait.json\n@@\n-invalid-draft\n*** End Patch"})},{type:"tool-result",name:"research_apply_patch",callId:"a",content:JSON.stringify({status:"complete",targets:[{path:"portrait.json",status:"applied",structuredResource:{resourceId:"synthetic",revision:1}}],fileDiffs:[native?{path:"portrait.json",comparison:"saved",beforeSaved:null,afterSaved:{saved:true}}:{path:"portrait.json",hunks:[{lines:["-invalid-draft"]}]}]})}])[0]!;}
function Fixture({row}:{row:ToolActivityRow}){const files=useStructuredPatchDiffs(row,toolPatchDiffFiles(row));return <output>{JSON.stringify(files)}</output>;}
async function mount(row:ToolActivityRow){const host=document.createElement("div"),root=createRoot(host);document.body.append(host);cleanups.push(async()=>{await act(async()=>root.unmount());host.remove();});await act(async()=>root.render(<Fixture row={row}/>));return {file:()=>JSON.parse(host.textContent??"[]")[0],rerender:async()=>act(async()=>root.render(<Fixture row={{...row}}/>))};}
it("reconstructs legacy tool counts from exact immutable saved revisions, never the current resource or repair draft",async()=>{
 const after={kind:"synthetic",saved:true};gateway.getVersion.mockResolvedValue({id:"synthetic",revision:1,content:after});
 const original=row(),data=original.result?.content;
 const f=await mount(original);expect({additions:f.file().additions,deletions:f.file().deletions}).toEqual(countWritingChangedLines("",JSON.stringify(after,null,2)));
 expect(gateway.getVersion).toHaveBeenCalledExactlyOnceWith("synthetic",1);expect(gateway.get).not.toHaveBeenCalled();
 await f.rerender();expect(gateway.getVersion).toHaveBeenCalledTimes(1);expect(original.result?.content).toBe(data);
});
it("uses native frozen snapshots without an additional historical read",async()=>{
 const f=await mount(row(true));expect(f.file().deletions).toBe(0);expect(gateway.getVersion).not.toHaveBeenCalled();
});
it("renders non-structured tools without resolving an unrelated domain gateway",async()=>{
 resolveGateway.mockImplementation(()=>{throw new Error("domain gateway unavailable");});
 const read=toolActivityRows([{type:"tool-call",name:"research_list_sources",callId:"read",content:"{}"},
  {type:"tool-result",name:"research_list_sources",callId:"read",content:'{"files":["main.tex"]}'}])[0]!;
 const f=await mount(read);expect(f.file()).toBeUndefined();expect(resolveGateway).not.toHaveBeenCalled();
});
it("shows the existing omission diagnostic when a real structured target has no gateway",async()=>{
 resolveGateway.mockImplementation(()=>{throw new Error("domain gateway unavailable");});
 const f=await mount(row());expect(f.file().omitted).toBe(true);expect(f.file().omissionReason).toContain("domain gateway unavailable");
 expect(f.file().lines).toEqual([]);expect(gateway.getVersion).not.toHaveBeenCalled();
});
it("shows a missing-snapshot diagnostic instead of falling back to misleading draft counts",async()=>{
 gateway.getVersion.mockRejectedValue({code:"not_found",message:"synthetic historical version missing"});
 const f=await mount(row());expect(f.file().omitted).toBe(true);expect(f.file().omissionReason).toContain("not_found");expect(f.file().lines).toEqual([]);
});
it("compares a later saved revision to its immediate predecessor, including pre-flattening legacy messages",async()=>{
 const before={saved:"old"},after={saved:"new"};
 gateway.getVersion.mockImplementation(async(id:string,revision:number)=>({id,revision,content:revision===1?before:after}));
 const original=row(),receipt=JSON.parse(original.result!.content);const target=receipt.targets[0];
 target.message=JSON.stringify({...target.structuredResource,revision:2});delete target.structuredResource;
 const f=await mount({...original,result:{...original.result!,content:JSON.stringify(receipt)}});
 expect(f.file().additions).toBe(1);expect(f.file().deletions).toBe(1);
 expect(gateway.getVersion.mock.calls).toEqual([["synthetic",2],["synthetic",1]]);expect(gateway.get).not.toHaveBeenCalled();
});
