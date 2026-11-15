import {expect,it} from "vitest";
import {toolActivityRows} from "./toolActivityModel";
import {toolPatchDiffFiles} from "./toolPatchDiffModel";
import {countWritingChangedLines} from "./writingChangeStats";
function row(patch:string,result:unknown={status:"applied"}) {return toolActivityRows([{type:"tool-call",name:"research_apply_patch",callId:"a",content:JSON.stringify({patch})},{type:"tool-result",name:"research_apply_patch",callId:"a",content:JSON.stringify(result)}])[0]!;}
const patch="*** Begin Patch\n*** Update File: src/one.ts\n@@\n const context = true;\n-old🧪\n+new🧪\n*** Add File: notes/new.md\n+new file\n*** End Patch";
it("separates files, counts actual +/- rows, and never invents positions for historical snippets",()=>{
 const files=toolPatchDiffFiles(row(patch));
 expect(files.map(file=>[file.name,file.additions,file.deletions])).toEqual([["one.ts",1,1],["new.md",1,0]]);
 expect(files[0]?.lines[0]?.newLine).toBeUndefined();expect(files[1]?.lines[0]?.newLine).toBe(1);
 expect(files[0]?.patch).not.toContain("notes/new.md");expect(files[0]?.patch).toContain("old🧪");
});
it("uses only frozen matching receipt paths and independent before/after counters",()=>{
 const files=toolPatchDiffFiles(row(patch,{status:"complete",targets:[{path:"src/one.ts",status:"applied"},{path:"notes/new.md",status:"applied"}],fileDiffs:[{path:"src/one.ts",hunks:[{oldStart:28,newStart:30,lines:[" old context","-old🧪","+new🧪"," after"]}]},{path:"other.ts",hunks:[{oldStart:90,newStart:90,lines:["+wrong"]}]}]}));
 expect(files[0]?.lines.map(line=>[line.kind,line.oldLine,line.newLine])).toEqual([["context",28,30],["deleted",29,undefined],["added",undefined,31],["context",30,32]]);
 expect(files.some(file=>file.path==="other.ts")).toBe(false);
 expect(toolPatchDiffFiles({...row(patch),status:"error"})).toEqual([]);
});
it("preserves copyable source for omitted previews without presenting false zero-line changes",()=>{
 const files=toolPatchDiffFiles(row(patch,{status:"applied",fileDiffs:[{path:"src/one.ts",hunks:[],omitted:true}]}));
 expect(files[0]?.omitted).toBe(true);expect(files[0]?.lines).toEqual([]);expect(files[0]?.patch).toContain("old🧪");
});
it.each([
 {path:"source://notes/note-id",name:"研究%20/方案",expected:"研究%20/方案"},
 {path:"chapters/result%20notes.tex",expected:"result%20notes.tex"},
 {path:"source://writings/project/result%20notes.tex",expected:"result notes.tex"},
])("preserves explicit resource names and only decodes source-address fallbacks %j",value=>{
 const input=`*** Begin Patch\n*** Update File: ${value.path}\n@@\n-old\n+new\n*** End Patch`;
 const files=toolPatchDiffFiles(row(input,{status:"complete",targets:[{path:value.path,name:value.name,status:"applied"}]}));
 expect(files[0]?.name).toBe(value.expected);expect(files[0]?.path).toBe(value.path);
});
it("uses the frozen preview name when the target receipt has no display name",()=>{
 const files=toolPatchDiffFiles(row(patch,{status:"applied",fileDiffs:[{path:"src/one.ts",name:"原始%20/标题",hunks:[]}]}));
 expect(files[0]?.name).toBe("原始%20/标题");
});
it("compares the saved versions rather than an invalid repair draft, with the same counts as the change card",()=>{
 const input="*** Begin Patch\n*** Update File: portrait.json\n@@\n-    draftOnly\n-    removed1\n-    removed2\n-    removed3\n*** End Patch";
 const after={kind:"synthetic_portrait",claims:[{id:"first",text:"Saved evidence"}]};
 const receipt={status:"complete",targets:[{path:"portrait.json",status:"applied"}],fileDiffs:[{path:"portrait.json",comparison:"saved",beforeSaved:null,afterSaved:after}]};
 const file=toolPatchDiffFiles(row(input,receipt))[0]!;
 expect({additions:file.additions,deletions:file.deletions}).toEqual(countWritingChangedLines("",JSON.stringify(after,null,2)));
 expect(file.additions).toBeGreaterThan(4);expect(file.deletions).toBe(0);
 expect(file.lines.some(line=>line.text.includes("draftOnly"))).toBe(false);
 expect(file.patch).toContain("draftOnly"); // Original input remains available for copying/raw inspection.
 const next={...after,claims:[{id:"first",text:"Updated evidence"}]};
 const second=toolPatchDiffFiles(row(input,{...receipt,fileDiffs:[{path:"portrait.json",comparison:"saved",beforeSaved:after,afterSaved:next}]}))[0]!;
 expect({additions:second.additions,deletions:second.deletions}).toEqual(countWritingChangedLines(JSON.stringify(after,null,2),JSON.stringify(next,null,2)));
 expect(second.additions).toBe(1);expect(second.deletions).toBe(1);
 expect(toolPatchDiffFiles(row(input,receipt))[0]).toEqual(file);
});
