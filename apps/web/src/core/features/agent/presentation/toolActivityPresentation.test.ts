import {expect,it} from "vitest";
import {toolActivityRows} from "./toolActivityModel";
import {toolCommand,toolCommandOutput,toolCompactTitle,toolExpandedTitle,toolPatch,toolPatchApplied,toolPatchOutcome} from "./toolActivityPresentation";
function row(args:unknown,result:unknown,name="research_exec_command") {return toolActivityRows([{type:"tool-call",callId:"a",name,content:JSON.stringify(args)},
 {type:"tool-result",callId:"a",name,content:JSON.stringify(result)}])[0]!;}
it("shows the exact command in compact text rather than the generic tool name",()=>{
 const value=row({cmd:"env PATH=/safe/bin pnpm check"},{exitCode:0,stdout:{head:"passed\n",tail:"",omittedBytes:0},stderr:{head:"",tail:"",omittedBytes:0}});
 expect(toolCommand(value)).toBe("env PATH=/safe/bin pnpm check");
 expect(toolCompactTitle(value)).toBe("已运行 env PATH=/safe/bin pnpm check");
 expect(toolCommandOutput(value)).toEqual({text:"passed\n",exitCode:0});
});
it("preserves retained output omissions, stderr, terminal output and nonzero exit codes",()=>{
 expect(toolCommandOutput(row({command:"check"},{exit_code:2,stdout:{head:"begin",tail:"end",omittedBytes:12},stderr:"failed"})))
  .toEqual({text:"begin\n…（工具已省略 12 字节）…\nend\nstderr:\nfailed",exitCode:2});
 expect(toolCommandOutput(row({cmd:"tty"},{terminalOutput:{head:"combined",tail:"",omittedBytes:0},stdout:"not separate"}))?.text).toBe("combined");
});
it("retains original patch contents and derives file paths only from its explicit headers",()=>{
 const patch="*** Begin Patch\n*** Update File: app.ts\n@@\n-old\n+new\n*** End Patch";
 const value=row({patch},{status:"ok"},"research_apply_patch");
 expect(toolPatch(value)).toBe(patch);expect(toolCompactTitle(value)).toBe("已编辑 app.ts");
 expect(toolPatch(row({query:"+fake"},{},"research_search_sources"))).toBeUndefined();
});
it("uses applied receipts for structured edits and preserves the actual invocation on failure",()=>{
 const args={operations:[{path:"main.tex",before:"old",after:"new"}]};
 const applied=row(args,{status:"applied",changes:[{path:"main.tex",kind:"modified"}]},"research_apply_patch");
 expect(toolPatchApplied(applied)).toBe(true);
 expect(toolCompactTitle(applied)).toBe("已编辑 main.tex");
 expect(toolExpandedTitle(applied)).toBe("已编辑 main.tex");
 const failed={...applied,status:"error" as const,result:{...applied.result!,content:'{"error":"patch_conflict"}'}};
 expect(toolPatchApplied(failed)).toBe(false);
 expect(toolCompactTitle(failed)).toBe("无法编辑 main.tex");
 expect(toolExpandedTitle(failed)).toBe(toolCompactTitle(failed));
});
it.each([
 {status:"failed"}, {status:"unknown"}, {status:"proposed"}, {},
 {status:"complete",targets:[]},
 {status:"complete",targets:[{path:"a",status:"proposed"}]},
 {status:"partial",targets:[{path:"a",status:"applied"},{path:"b",status:"failed"}]},
 {status:"applied",approvalRequired:true},
])("does not claim an edit succeeded for an unconfirmed receipt %j",result=>{
 const value=row({patch:"*** Begin Patch\n*** Update File: a\n@@\n-old\n+new\n*** End Patch"},result,"research_apply_patch");
 expect(toolPatchApplied(value)).toBe(false);
 if(result.status==="failed"||result.status==="partial")expect(toolCompactTitle(value)).toBe(result.status==="partial"?"无法编辑 a、b":"无法编辑 a");
 else expect(toolCompactTitle(value)).toMatch(/^已运行 research_apply_patch /u);
});
it("shows the read filename in both disclosures and adds only actual partial-read line ranges",()=>{
 const full=row({path:"source://writings/project/file"},{name:"main.tex",startChar:0,nextChar:null,startLine:1,endLine:30},"research_read_file");
 expect(toolCompactTitle(full)).toBe("已读取 main.tex");
 expect(toolExpandedTitle(full)).toBe("已读取 main.tex");
 const partial=row({path:"main.tex",startChar:12,maxChars:100},{startChar:12,nextChar:112,startLine:3,endLine:8},"research_read_file");
 expect(toolCompactTitle(partial)).toBe("已读取 main.tex (line 3 to 8)");
 expect(toolExpandedTitle(partial)).toBe(toolCompactTitle(partial));
 expect(toolCompactTitle(row({path:"main.tex"},{startChar:0,nextChar:100,startLine:1,endLine:5},"research_read_file")))
  .toBe("已读取 main.tex (line 1 to 5)");
 expect(toolCompactTitle({...partial,status:"error"})).toBe('已运行 research_read_file {"path":"main.tex","startChar":12,"maxChars":100}');
 expect(toolCompactTitle({...partial,status:"interrupted"})).toBe("读取中断 main.tex");
});
it.each([{startChar:20,nextChar:40},{startLine:0,endLine:3},{startLine:5,endLine:4},{startLine:1.5,endLine:3}])("does not guess read lines from missing or invalid metadata %j",result=>{
 expect(toolCompactTitle(row({path:"old.md",startChar:20,maxChars:20},result,"research_read_file"))).toBe("已读取 old.md");
});
it("shows the literal query and authorized scope in both search titles, even with zero matches",()=>{
 const value=row({query:"^Running|^Doc-tests|^test result:",sourceId:"release-v0.5.0.log"},{total:0,matches:[]},"research_search_sources");
 const title="已搜索 ^Running|^Doc-tests|^test result:（资源）";
 expect(toolCompactTitle(value)).toBe(title);expect(toolExpandedTitle(value)).toBe(title);
 expect(toolCompactTitle({...value,status:"error"})).toBe(title.replace("已搜索","搜索失败"));
 expect(toolCompactTitle({...value,status:"interrupted"})).toBe(title.replace("已搜索","搜索中断"));
 expect(toolCompactTitle(row({query:"证据"},{matches:[{sourceId:"matched.md"}]},"research_search_sources")))
  .toBe("已搜索 证据");
});
it("preserves every explicit search filter without inferring scope from matching results",()=>{
 const value=row({query:"🧪 phrase",resourceId:"project-id",nameQuery:"Research",pathGlob:"chapters/*.tex",resourceType:"manuscriptFile",offset:20},
  {matches:[{sourceId:"only-one-hit"}]},"research_search_sources");
 expect(toolCompactTitle(value)).toBe("已搜索 🧪 phrase（资源，名称 Research，路径 chapters/*.tex，类型 manuscriptFile）");
 expect(toolCompactTitle(row({query:"word",fileId:"source://library/paper/pdf"},{},"research_search_sources")))
  .toBe("已搜索 word（资源）");
 expect(toolCompactTitle(row({query:"legacy"},{},"search_sources"))).toBe("已搜索 legacy");
 expect(toolCompactTitle(row({},{},"research_search_sources"))).toBe("搜索资料");
});
it("names the frozen loaded skill in compact and expanded headers without claiming failed loads succeeded",()=>{
 const value=row({identity:{source_id:".agents/skills/context/SKILL.md"}},{skill:{name:"User Context"}},"research_load_skill");
 expect(toolCompactTitle(value)).toBe("已读取 User Context 技能");
 expect(toolExpandedTitle(value)).toBe("已读取 User Context 技能");
 expect(toolCompactTitle({...value,status:"error"})).toContain("已运行 research_load_skill");
});

it.each([["completed","已编辑"],["failed","无法编辑"],["declined","拒绝编辑"]] as const)("distinguishes a %s edit receipt from tool transport success",(status,label)=>{
 const value=row({path:"main.tex"},{status,message:"original reason",code:"test_code",operationId:"op-1"},"research_apply_patch");
 expect(toolPatchOutcome(value)).toBe(status);
 expect(toolCompactTitle(value)).toBe(`${label} main.tex`);
 expect(toolExpandedTitle(value)).toBe(`${label} main.tex`);
 expect(toolPatchApplied(value)).toBe(status==="completed");
 expect(toolPatchOutcome({...value,status:"interrupted"})).toBeUndefined();
});
it("never treats permission failures or unconfirmed edits as an explicit refusal",()=>{
 for(const status of ["unknown","proposed"])expect(toolPatchOutcome(row({path:"a"},{status},"research_apply_patch"))).toBeUndefined();
 const failed=row({path:"a"},{error:"permission_denied",message:"sandbox denied write"},"research_apply_patch");
 expect(toolPatchOutcome(failed)).toBe("failed");
 expect(toolPatchOutcome({...failed,result:{...failed.result!,content:JSON.stringify({status:"declined",code:"edit_declined"})},status:"error"})).toBe("declined");
});
it("recognizes explicit legacy rejection receipts without treating ordinary target failures as rejection",()=>{
 const receipt=(targets:unknown[])=>row({path:"a"},{status:"failed",targets},"research_apply_patch");
 const declined={path:"a",status:"failed",message:JSON.stringify({code:"edit_declined",message:"用户拒绝"})};
 expect(toolPatchOutcome(receipt([declined]))).toBe("declined");
 expect(toolPatchOutcome(row({path:"a"},{status:"rejected"},"research_apply_patch"))).toBe("declined");
 expect(toolPatchOutcome(receipt([declined,{path:"b",status:"failed",message:JSON.stringify({code:"permission_denied"})}]))).toBe("failed");
 expect(toolPatchOutcome(receipt([{path:"a",status:"failed",message:"rejected-looking prose; patch context does not match"}]))).toBe("failed");
});
it("recognizes legacy structured validation rejection while preserving technical failures and mixed outcomes",()=>{
 const target=(code:string)=>({path:"source://notes/synthetic-resource",status:"failed",message:"Draft retained",structuredResource:{status:"invalid",resourceId:"synthetic-resource",draftId:"synthetic-draft",errors:[{code,path:"$.field",message:"synthetic reason"}]}});
 const outcome=(targets:unknown[])=>toolPatchOutcome(row({path:"source://notes/synthetic-resource"},{status:"failed",targets},"research_apply_patch"));
 for(const code of ["invalid_json","invalid_type","invalid_content","invalid_manual_content","invalid_evidence","template_mismatch"])expect(outcome([target(code)])).toBe("declined");
 const beforeFlatten=target("invalid_evidence");
 expect(outcome([{...beforeFlatten,structuredResource:undefined,message:JSON.stringify(beforeFlatten.structuredResource)}])).toBe("declined");
 for(const code of ["storage_error","permission_denied","revision_conflict","unknown_future_error"])expect(outcome([target(code)])).toBe("failed");
 expect(outcome([target("invalid_evidence"),target("storage_error")])).toBe("failed");
 expect(outcome([target("invalid_evidence"),{path:"b",status:"applied"}])).toBe("failed");
});
