import {expect,it} from "vitest";
import {toolActivityRows,type ToolActivityRow} from "./toolActivityModel";
import {mergeToolReadRows,toolReadDescription} from "./toolReadActivity";
function read(id:string,start:number,end:number,next:number|null,extra:Record<string,unknown>={}):ToolActivityRow {
 return toolActivityRows([{type:"tool-call",name:"research_read_source",callId:id,content:JSON.stringify({fileId:"file",startChar:start,view:"structured"})},
 {type:"tool-result",name:"research_read_source",callId:id,content:JSON.stringify({fileId:"file",sourceUrl:"source://library/paper/file",name:"paper.pdf",readViewId:"snapshot",startChar:start,endChar:end,totalChars:12,nextChar:next,startLine:start?3:1,endLine:start?4:2,...extra})}])[0]!;
}
it("merges consecutive windows of one source into a full read without losing the exact envelopes",()=>{
 const first=read("first",0,6,6),last=read("last",6,12,null),rows=mergeToolReadRows([first,last]);
 expect(rows).toHaveLength(1);expect(rows[0]?.reads).toEqual([first,last]);
 expect(toolReadDescription(rows[0]!)?.name).toBe("paper.pdf");expect(toolReadDescription(rows[0]!)?.range).toBe("");
 expect(toolReadDescription(first)?.range).toBe(" (line 1 to 2)");
});
it("does not merge failures, different snapshots/views or reads separated by another tool",()=>{
 const first=read("a",0,6,6),second=read("b",6,12,null);
 expect(mergeToolReadRows([first,{...second,status:"error"}])).toHaveLength(2);
 expect(mergeToolReadRows([first,read("b",6,12,null,{readViewId:"changed"})])).toHaveLength(2);
 expect(mergeToolReadRows([first,{...second,call:{...second.call!,content:'{"fileId":"file","view":"raw"}'}}])).toHaveLength(2);
 const other={...second,call:{...second.call!,name:"research_search_sources"}};
 expect(mergeToolReadRows([first,other,second])).toHaveLength(3);
});
it("keeps non-contiguous line ranges honest and merges legacy continuation offsets without inventing line positions",()=>{
 const a=read("a",0,3,3,{startLine:1,endLine:1}),b=read("b",9,12,null,{startLine:5,endLine:6});
 expect(toolReadDescription(mergeToolReadRows([a,b])[0]!)?.range).toBe(" (line 1 to 1, 5 to 6)");
 const legacyA=read("a",0,6,6,{readViewId:null,startLine:null,endLine:null}),legacyB=read("b",6,12,null,{readViewId:null,startLine:null,endLine:null});
 expect(mergeToolReadRows([legacyA,legacyB])).toHaveLength(1);expect(toolReadDescription(mergeToolReadRows([legacyA,legacyB])[0]!)?.range).toBe("");
});
