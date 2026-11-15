import type {ToolActivityRow} from "./toolActivityModel";
function object(value:unknown):Record<string,unknown>|undefined{return value&&typeof value==="object"&&!Array.isArray(value)?value as Record<string,unknown>:undefined;}
function json(text:unknown):Record<string,unknown>|undefined{if(typeof text!=="string")return object(text);try{return object(JSON.parse(text));}catch{return;}}
/** Select diagnostics only. Arguments, patch text, resource bodies and raw
 * envelopes remain unchanged in the independent developer disclosure. */
export function toolEditDiagnostics(row:ToolActivityRow):readonly string[]{
 const result=json(row.result?.content),lines:string[]=[],seen=new Set<string>();
 function add(label:string,value:unknown){if((typeof value==="string"&&value.length>0)||typeof value==="number"){const text=String(value);if(!seen.has(text)){seen.add(text);lines.push(`${label}：${text}`);}}}
 function collect(value:Record<string,unknown>|undefined,depth=0):void{
  if(!value||depth>4)return;
  add("错误码",value.code);if(typeof value.path==="string"&&value.path.startsWith("$"))add("字段",value.path);
  const error=json(value.error);if(error)collect(error,depth+1);else add("错误码",value.error);
  const message=json(value.message);if(message)collect(message,depth+1);else add("原因",value.message);
  const cause=json(value.cause);if(cause)collect(cause,depth+1);else add("原因",value.cause);
  for(const [key,label] of [["requestId","请求编号"],["turnId","回合编号"],["operationId","操作编号"],["taskId","任务编号"],["changeId","修改编号"],["httpStatus","HTTP 状态"],["statusCode","HTTP 状态"]] as const)add(label,value[key]);
  if(Array.isArray(value.errors))for(const issue of value.errors)collect(object(issue),depth+1);
  if(value.draftId){add("草稿","已保留");add("草稿编号",value.draftId);add("草稿修订",value.draftRevision);}
  collect(object(value.receipt),depth+1);collect(object(value.structuredResource),depth+1);
  add("处理建议",value.guidance);
 }
 // A target's path is a resource address, not an error-field location.
 if(result){const {targets,...top}=result;collect(top);if(Array.isArray(targets))for(const target of targets){const entry=object(target);if(entry){const {path:_,name:__,...diagnostic}=entry;collect(diagnostic);}}}
 else add("原因",row.result?.content);
 return lines.length?lines:["未返回具体错误原因。"];
}
