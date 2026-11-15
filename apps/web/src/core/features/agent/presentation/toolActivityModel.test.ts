import {expect,it} from "vitest";
import {toolActivityRows,toolActivitySummary} from "./toolActivityModel";
import type {WritingAgentActivityProps as Item} from "./WritingAgentActivity";
const call=(id:string,name="research_read_file"):Item=>({type:"tool-call",callId:id,name,content:'{"path":"paper.md"}',streaming:true});
const result=(id:string,error=false):Item=>({type:"tool-result",callId:id,name:"research_read_file",content:"done",error});
it("pairs concurrent same-name calls by ID and keeps only unfinished calls running",()=>{
 const rows=toolActivityRows([call("a"),call("b"),result("b"),result("a",true),call("c")]);
 expect(rows.map(row=>row.status)).toEqual(["error","success","running"]);
 expect(rows[0]?.result?.callId).toBe("a");expect(rows[1]?.result?.callId).toBe("b");
});
it("summarizes the complete group deterministically, including failures and interrupted calls",()=>{
 const rows=toolActivityRows([call("a"),result("a"),call("b"),result("b",true),{...call("c","research_apply_patch"),streaming:false}]);
 expect(toolActivitySummary(rows)).toBe("编辑文件、读取文件");
});
it("merges operation summaries into four labels without changing individual titles or execution order",()=>{
 const names=["shell","research_compile_workspace","compile_latex","research_search_sources","research_find_sources",
  "research_list_sources","list_files","research_list_workspace","research_read_file","research_read_source","read_source","search_sources",
  "research_review_workspace_changes","research_read_workspace_change","research_deliver_files","apply_patch","research_apply_patch",
  "research_read_skill_resource","research_load_skill"];
 const rows=toolActivityRows(names.map((name,index)=>({...call(String(index),name),streaming:false})));
 expect(toolActivitySummary(rows)).toBe("加载工具、编辑文件、读取文件、运行命令");
 expect(rows.map(row=>row.call?.name)).toEqual(names);
 expect(rows.find(row=>row.call?.name==="research_load_skill")?.label).toBe("加载技能");
 expect(rows.find(row=>row.call?.name==="research_read_source")?.label).toBe("读取资料");
 expect(toolActivitySummary(rows.filter(row=>["research_search_sources","research_read_file","research_list_sources"].includes(row.call?.name??"")))).toBe("读取文件");
 expect(toolActivitySummary(rows.filter(row=>["compile_latex","research_compile_workspace","shell"].includes(row.call?.name??"")))).toBe("运行命令");
});
it("retains unknown tool names and the unnamed and empty group fallbacks",()=>{
 expect(toolActivitySummary(toolActivityRows([call("unknown","custom_tool")]))).toBe("custom_tool");
 expect(toolActivitySummary(toolActivityRows([{type:"tool-result",content:"ok"}]))).toBe("工具");
 expect(toolActivitySummary([])).toBe("工具调用");
});
it("never counts unconfirmed terminal calls as success and supports unkeyed historical pairs",()=>{
 expect(toolActivityRows([{...call("a"),streaming:false}])[0]?.status).toBe("interrupted");
 const old:Item={type:"tool-call",name:"shell",content:"{}",streaming:true};
 expect(toolActivityRows([old,{type:"tool-result",name:"shell",content:"ok"}])[0]?.status).toBe("success");
});
it("keeps an executing call active after its argument item completes and marks interrupted results honestly",()=>{
 const waiting={...call("a"),streaming:false,turnFinished:false};
 expect(toolActivityRows([waiting])[0]?.status).toBe("running");
 expect(toolActivityRows([{...waiting,turnFinished:true}])[0]?.status).toBe("interrupted");
 expect(toolActivityRows([waiting,{...result("a"),interrupted:true}])[0]?.status).toBe("interrupted");
});
