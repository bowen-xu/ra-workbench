/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import {ToolActivityGroupIcon,ToolActivityIcon} from "./ToolActivityIcon";
import {toolActivityRows,toolActivitySummary} from "./toolActivityModel";
vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
it("keeps the original headline icon priority after merging summary wording",async()=>{
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 const names=["research_exec_command","research_read_file","research_search_sources","research_apply_patch","research_load_skill"];
 const expected=[["tool","加载工具"],["edit","编辑文件"],["search","读取文件"],["read","读取文件"],["command","运行命令"]] as const;
 try{for(const [category,label] of expected){const rows=toolActivityRows(names.flatMap((name,index)=>[
  {type:"tool-call" as const,name,callId:String(index),content:"{}"},{type:"tool-result" as const,name,callId:String(index),content:"ok"}]));
  await act(async()=>root.render(<ToolActivityGroupIcon rows={rows}/>));
  expect(host.querySelector("[data-tool-kind]")?.getAttribute("data-tool-kind")).toBe(category);
  expect(toolActivitySummary(rows).startsWith(label)).toBe(true);names.pop();
 }}finally{await act(async()=>root.unmount());host.remove();}
});
it("uses the operation name rather than the search substring in the research namespace",async()=>{
 const host=document.createElement("div");const root=createRoot(host);
 try{for(const [name,category,icon] of [["research_read_file","read","book-open"],["research_search_sources","search","search"],["research_load_skill","tool","wrench"],
  ["research_list_sources","read","folder"],["research_list_workspace","read","folder"],["list_files","read","folder"]] as const){
  const [row]=toolActivityRows([{type:"tool-call",name,callId:name,content:"{}"},{type:"tool-result",name,callId:name,content:"ok"}]);
  await act(async()=>root.render(<ToolActivityIcon row={row!}/>));
  expect(host.querySelector("[data-tool-kind]")?.getAttribute("data-tool-kind")).toBe(category);
  expect(host.querySelector(`.lucide-${icon}`)).not.toBeNull();
 }}finally{await act(async()=>root.unmount());}
});
it("preserves folder, book and search group icons for the same reading summary",async()=>{
 const host=document.createElement("div");const root=createRoot(host);
 try{for(const [names,icon] of [
  [["research_list_sources","research_read_file"],"folder"],
  [["research_read_file","research_list_sources"],"book-open"],
  [["research_list_sources","research_search_sources","research_read_file"],"search"],
 ] as const){
  const rows=toolActivityRows(names.map((name,index)=>({type:"tool-result",name,callId:String(index),content:"ok"})));
  await act(async()=>root.render(<ToolActivityGroupIcon rows={rows}/>));
  expect(toolActivitySummary(rows)).toBe("读取文件");
  expect(host.querySelector(`.lucide-${icon}`)).not.toBeNull();
 }}finally{await act(async()=>root.unmount());}
});
