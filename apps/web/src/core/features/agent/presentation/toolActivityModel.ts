import type { WritingAgentActivityProps } from "./WritingAgentActivity";

const labels: Record<string,string> = {
 apply_patch:"应用修改",research_apply_patch:"应用修改",research_deliver_files:"保存文件修改",
 compile_latex:"编译 LaTeX",research_compile_workspace:"编译工作区",list_files:"查看文件",research_list_workspace:"查看文件",
 read_source:"读取资料",research_read_source:"读取资料",research_read_file:"读取文件",research_list_sources:"列出资料",research_find_sources:"查找资料",
 search_sources:"搜索资料",research_search_sources:"搜索资料",research_review_workspace_changes:"查看文件变更",research_read_workspace_change:"读取文件变更",
 shell:"运行命令",research_exec_command:"运行命令",research_load_skill:"加载技能",research_read_skill_resource:"读取技能资源",
};
export function toolLabel(name?:string):string {return name ? labels[name] ?? name : "工具";}
export interface ToolActivityRow {
 readonly key:string;
 readonly reads?:readonly ToolActivityRow[];
 readonly call?:WritingAgentActivityProps;
 readonly result?:WritingAgentActivityProps;
 readonly status:"running"|"success"|"error"|"interrupted";
 readonly label:string;
 readonly detail:string;
}
function detail(item:WritingAgentActivityProps):string {
 try {
  const args:unknown=JSON.parse(item.content);
  if(!args || typeof args!=="object")return "";
  const record=args as Record<string,unknown>;
  for(const key of ["path","file","sourceUrl","source","query","command","cmd","skillName","name"]){
   if(typeof record[key]==="string")return record[key];
  }
 }catch{ /* Raw payload remains available in the disclosure. */ }
 return "";
}
/** Call IDs take precedence; legacy unkeyed records pair FIFO within one name. */
export function toolActivityRows(items:readonly WritingAgentActivityProps[]):ToolActivityRow[] {
 const rows:{call?:WritingAgentActivityProps;result?:WritingAgentActivityProps;key:string}[]=[];
 for(const [index,item] of items.entries()) {
  if(item.type==="tool-call")rows.push({call:item,key:item.callId ?? `call:${index}`});
  else {
   const row=rows.find(row=>row.call && !row.result && (item.callId!==undefined
    ? row.call.callId===item.callId : row.call.callId===undefined && row.call.name===item.name));
   if(row)row.result=item;else rows.push({result:item,key:`result:${item.callId ?? index}`});
  }
 }
 return rows.map(row=>{
  const item=row.call ?? row.result!;
  const status=row.result?.error || row.call?.error || row.call?.laterResult === "error" ? "error" : row.result?.interrupted || row.call?.laterResult === "interrupted" || !row.result && row.call?.interrupted && !row.call?.laterResult ? "interrupted"
   : row.result?.streaming || !row.result && row.call?.turnFinished!==true && (row.call?.streaming || row.call?.turnFinished===false)
   && !row.call?.laterResult ? "running" : row.result || row.call?.laterResult === "success" ? "success" : "interrupted";
  return {...row,status,label:toolLabel(item.name),detail:detail(item)};
 });
}
// Summary wording is independent of per-operation titles and icon priority.
const summaryLabels: Record<string,string> = {
 load_skill:"加载工具",read_skill_resource:"加载工具",
 apply_patch:"编辑文件",deliver_files:"编辑文件",
 find_sources:"读取文件",search_sources:"读取文件",list_files:"读取文件",list_workspace:"读取文件",list_sources:"读取文件",
 read_file:"读取文件",read_source:"读取文件",review_workspace_changes:"读取文件",read_workspace_change:"读取文件",
 compile_latex:"运行命令",compile_workspace:"运行命令",shell:"运行命令",exec_command:"运行命令",
};
const summaryOrder = ["加载工具","编辑文件","读取文件","运行命令"];

/** Deterministic activity summary; no model call and no claim of unconfirmed success. */
export function toolActivitySummary(rows:readonly ToolActivityRow[]):string {
 const labels=[...new Set(rows.map(row=>{
  const name=(row.call?.name??row.result?.name??"").replace(/^research_/u,"");
  return summaryLabels[name]??row.label;
 }))];
 labels.sort((a,b)=>summaryOrder.indexOf(a)-summaryOrder.indexOf(b));
 return labels.join("、") || "工具调用";
}
