import {toolReadDescription} from "./toolReadActivity";
import {toolDiscoveryText} from "./toolDiscoveryActivity";
import type {ToolActivityRow} from "./toolActivityModel";
function record(content:string|undefined):Record<string,unknown>|undefined {
 try {const value:unknown=JSON.parse(content??"");return value&&typeof value==="object"&&!Array.isArray(value)?value as Record<string,unknown>:undefined;}catch{return undefined;}
}
export function toolCommand(row:ToolActivityRow):string|undefined {
 if(!["shell","research_exec_command","exec_command"].includes(row.call?.name??""))return;
 const args=record(row.call?.content);
 for(const key of ["command","cmd"])if(typeof args?.[key]==="string")return args[key];
 // Legacy shell inputs may already be literal command text.
 if(row.call?.content.trim()&&!args)return row.call.content;
 return;
}
export function toolPatch(row:ToolActivityRow):string|undefined {
 if(!["apply_patch","research_apply_patch"].includes(row.call?.name??""))return;
 const args=record(row.call?.content);
 for(const key of ["patch","input"])if(typeof args?.[key]==="string")return args[key];
 if(row.call?.content.startsWith("*** Begin Patch"))return row.call.content;
 return;
}
function isPatch(row:ToolActivityRow):boolean {
 return ["apply_patch","research_apply_patch"].includes(row.call?.name??row.result?.name??"");
}
/** A returned tool call can contain a rejected, proposed or uncertain edit. */
export function toolPatchApplied(row:ToolActivityRow):boolean {
 if(!isPatch(row)||row.status!=="success"||!row.result)return false;
 const result=record(row.result.content);
 if(!result)return /^Success\b/u.test(row.result.content);
 if(result.error!==undefined)return false;
 if(result.approvalRequired===true)return false;
 if(result.status!==undefined&&!["applied","ok","success","complete","completed"].includes(String(result.status)))return false;
 if(Array.isArray(result.targets))return result.targets.length>0&&result.targets.every(target=>
  !!target&&typeof target==="object"&&"status" in target&&target.status==="applied");
 return ["applied","ok","success","complete","completed"].includes(String(result.status));
}
/** Transport success does not imply an edit succeeded. Pending and interrupted
 * calls retain their original state rather than inventing a terminal outcome. */
export function toolPatchOutcome(row:ToolActivityRow):"completed"|"failed"|"declined"|undefined {
 if(!isPatch(row)||!row.result||row.status==="running"||row.status==="interrupted")return;
 const result=record(row.result.content);
 const declined=(value:Record<string,unknown>|undefined)=>value?.status==="declined"||value?.status==="rejected"||value?.code==="edit_declined"||value?.code==="edit_rejected";
 const validationDeclined=(value:Record<string,unknown>|undefined)=>value?.status==="invalid"&&typeof value.resourceId==="string"&&Array.isArray(value.errors)&&value.errors.length>0&&value.errors.every(issue=>
  issue&&typeof issue==="object"&&"code" in issue&&["invalid_json","invalid_type","invalid_content","invalid_manual_content","invalid_evidence","template_mismatch"].includes(String(issue.code)));
 const declinedTarget=(value:unknown)=>{
  if(!value||typeof value!=="object")return false;
  const target=value as Record<string,unknown>;
  const message=record(typeof target.message==="string"?target.message:undefined);
  const structured=target.structuredResource&&typeof target.structuredResource==="object"?target.structuredResource as Record<string,unknown>:undefined;
  return declined(target)||declined(message)||validationDeclined(structured)||validationDeclined(message);
 };
 if(declined(result)||Array.isArray(result?.targets)&&result.targets.length>0&&result.targets.every(declinedTarget))return "declined";
 const failedTarget=Array.isArray(result?.targets)&&result.targets.some(target=>target&&typeof target==="object"&&"status" in target&&target.status==="failed");
 if(row.status==="error"||result?.status==="failed"||result?.status==="partial"||result?.error!==undefined||failedTarget)return "failed";
 if(toolPatchApplied(row))return "completed";
 return;
}
export function toolPatchTargets(row:ToolActivityRow):readonly {path:string;name?:string}[] {
 const result=record(row.result?.content),args=record(row.call?.content);
 const entries=result?.targets??result?.changes??args?.operations;
 if(Array.isArray(entries)){
  const targets=entries.flatMap(entry=>entry&&typeof entry==="object"&&"path" in entry&&typeof entry.path==="string"?[{path:entry.path,...("name" in entry&&typeof entry.name==="string"?{name:entry.name}:{})}]:[]);
  if(targets.length)return targets;
 }
 const paths=[...(toolPatch(row)??"").matchAll(/^\*\*\* (?:Update|Add|Delete) File: (.+)$/gmu)].map(match=>({path:match[1]!}));
 return paths.length?paths:typeof args?.path==="string"?[{path:args.path}]:[];
}
function patchPaths(row:ToolActivityRow):string[]{return toolPatchTargets(row).map(target=>target.path);}
function isReadFile(row:ToolActivityRow):boolean {
 return (row.call?.name??row.result?.name)==="research_read_file";
}
/** Failed reads use the same inspectable invocation/output presentation as commands. */
export function toolShellInvocation(row:ToolActivityRow):string|undefined {
 const command=toolCommand(row);
 if(command)return command;
 if(row.call&&((isPatch(row)&&!toolPatchApplied(row))||row.status==="error"))return `${row.call.name} ${row.call.content}`;
 return;
}
function readFileTitle(row:ToolActivityRow):string {
 if(row.status==="error"&&row.call)return `已运行 ${toolShellInvocation(row)}`;
 const result=record(row.result?.content),args=record(row.call?.content);
 const name=[result?.name,args?.path,result?.path,row.detail].find(value=>typeof value==="string"&&value.length>0)??"文件";
 const label={success:"已读取",error:"读取失败",interrupted:"读取中断",running:"正在读取"}[row.status];
 const start=result?.startLine,end=result?.endLine;
 const wholeFile=result?.startChar===0&&result?.nextChar===null;
 const range=row.status==="success"&&!wholeFile&&typeof start==="number"&&Number.isSafeInteger(start)&&start>=1
  &&typeof end==="number"&&Number.isSafeInteger(end)&&end>=start?` (line ${start} to ${end})`:"";
 return `${label} ${name}${range}`;
}
function isSearch(row:ToolActivityRow):boolean {
 return ["research_search_sources","search_sources"].includes(row.call?.name??row.result?.name??"");
}
function skillTitle(row:ToolActivityRow):string|undefined {
 if((row.call?.name??row.result?.name)!=="research_load_skill")return;
 if(row.status==="error"&&row.call)return `已运行 ${toolShellInvocation(row)}`;
 const args=record(row.call?.content),result=record(row.result?.content),skill=record(result?.skill?JSON.stringify(result.skill):undefined);
 const identity=args?.identity&&typeof args.identity==="object"?args.identity as Record<string,unknown>:undefined;
 const name=[skill?.name,args?.skillName,args?.name,identity?.source_id].find(value=>typeof value==="string"&&value.length>0);
 const label={success:"已读取",error:"读取失败",interrupted:"读取中断",running:"正在读取"}[row.status];
 return `${label}${name?` ${name} `:""}技能`;
}
export function toolCompactTitle(row:ToolActivityRow):string {
 const discovery=toolDiscoveryText(row);if(discovery)return discovery;
 if(row.status==="running")return [row.label,row.detail].filter(Boolean).join(" · ");
 const read=toolReadDescription(row);if(read)return `已读取 ${read.name}${read.range}`;
 const skill=skillTitle(row);if(skill)return skill;
 if(isReadFile(row))return readFileTitle(row);
 const command=toolCommand(row);
 if(command)return `${row.status==="error"?"运行失败":row.status==="interrupted"?"运行中断":"已运行"} ${command}`;
 if(isPatch(row)) {
  const outcome=toolPatchOutcome(row);
  if(outcome)return `${{completed:"已编辑",failed:"无法编辑",declined:"拒绝编辑"}[outcome]} ${patchPaths(row).join("、")||row.detail||"文件"}`;
  return row.call?`已运行 ${row.call.name} ${row.call.content}`:row.label;
 }
 return [row.label,row.detail].filter(Boolean).join(" · ");
}
export function toolExpandedTitle(row:ToolActivityRow):string {
 return isPatch(row)||isReadFile(row)||isSearch(row)||toolDiscoveryText(row)||toolReadDescription(row)||skillTitle(row)?toolCompactTitle(row):row.label;
}
function retainedText(value:unknown):string|undefined {
 if(typeof value==="string")return value;
 if(!value || typeof value!=="object")return;
 const output=value as Record<string,unknown>;
 if(typeof output.head!=="string")return;
 const tail=typeof output.tail==="string"?output.tail:"";
 const omitted=typeof output.omittedBytes==="number"?output.omittedBytes:typeof output.omitted_bytes==="number"?output.omitted_bytes:0;
 return output.head+(omitted>0?`\n…（工具已省略 ${omitted} 字节）…\n`:"")+tail;
}
/** Project known process envelopes faithfully; the raw record is always retained. */
export function toolCommandOutput(row:ToolActivityRow):{text:string;exitCode?:number}|undefined {
 const output=record(row.result?.content);
 if(!output)return row.result?{text:row.result.content}:undefined;
 const terminal=retainedText(output.terminalOutput??output.terminal_output);
 const stdout=retainedText(output.stdout),stderr=retainedText(output.stderr);
 const combined=retainedText(output.output??output.aggregatedOutput??output.aggregated_output);
 const text=terminal??combined??(stdout!==undefined||stderr!==undefined?[stdout,stderr?`stderr:\n${stderr}`:undefined].filter(value=>value!==undefined).join("\n"):undefined);
 const exit=output.exitCode??output.exit_code;
 return text!==undefined?{text,...(typeof exit==="number"?{exitCode:exit}:{})}:undefined;
}
