import type {ToolActivityRow} from "./toolActivityModel";
function record(text:string|undefined):Record<string,unknown>{try{const value:unknown=JSON.parse(text??"");return value&&typeof value==="object"&&!Array.isArray(value)?value as Record<string,unknown>:{};}catch{return {};}}
function text(...values:unknown[]):string|undefined{return values.find(value=>typeof value==="string"&&value.length>0) as string|undefined;}
function number(value:unknown):number|undefined{return typeof value==="number"&&Number.isSafeInteger(value)&&value>=0?value:undefined;}
export function toolReadReceipt(row:ToolActivityRow){
 if(!["research_read_file","research_read_source","read_source","read_file"].includes(row.call?.name??row.result?.name??"")||row.status!=="success")return;
 const args=record(row.call?.content),result=record(row.result?.content),anchor=record(typeof result.anchor==="string"?result.anchor:undefined);
 const path=text(result.sourceUrl,result.path,args.path),name=text(result.name,result.title,args.path,result.path);
 const identity=text(path,result.fileId,args.fileId,result.sourceId,args.sourceId);
 const start=number(result.startChar)??number(args.startChar)??0;
 const end=number(result.endChar)??number(result.nextChar)??(typeof result.text==="string"?start+Array.from(result.text).length:undefined);
 const total=number(result.totalChars)??(result.nextChar===null?end:undefined);
 return {path,name,identity,view:text(args.view,result.view)??"raw",version:text(result.readViewId,result.version,result.revision,anchor.sourceSnapshotId),start,end,total,
  complete:start===0&&result.nextChar===null,startLine:number(result.startLine),endLine:number(result.endLine)};
}
/** Adjacent successful reads share one display row, while retaining every exact
 * call/result. An intervening tool, failure, view or snapshot change breaks it. */
export function mergeToolReadRows(rows:readonly ToolActivityRow[]):ToolActivityRow[]{
 const output:ToolActivityRow[]=[];
 for(const row of rows){const last=output.at(-1),a=last&&toolReadReceipt(last),b=toolReadReceipt(row);
  const tail=last?.reads?.at(-1)??last,previous=tail&&toolReadReceipt(tail);
  const sameSnapshot=a&&b&&(a.version||b.version?a.version===b.version:previous?.end===b.start||a.complete&&b.complete&&last?.result?.content===row.result?.content);
  if(last&&a?.identity&&b?.identity===a.identity&&a.view===b.view&&sameSnapshot){output[output.length-1]={...last,reads:[...(last.reads??[last]),row]};}
  else output.push(row);
 }
 return output;
}
export function toolReadDescription(row:ToolActivityRow){
 const receipt=toolReadReceipt(row);if(!receipt)return;
 const parts=(row.reads??[row]).map(toolReadReceipt).filter(value=>value!==undefined);
 const spans=parts.filter(part=>part.end!==undefined).sort((a,b)=>a.start-b.start);let covered=0;
 for(const span of spans){if(span.start>covered)break;covered=Math.max(covered,span.end!);}
 const total=parts.find(part=>part.total!==undefined)?.total;
 const whole=parts.some(part=>part.complete)||(total!==undefined&&covered>=total);
 const ranges=parts.every(part=>part.startLine!==undefined&&part.startLine>=1&&part.endLine!==undefined&&part.endLine>=part.startLine)
  ?parts.map(part=>({start:part.startLine!,end:part.endLine!})).sort((a,b)=>a.start-b.start):[];
 const merged:{start:number;end:number}[]=[];
 for(const range of ranges){const last=merged.at(-1);if(last&&range.start<=last.end+1)last.end=Math.max(last.end,range.end);else merged.push({...range});}
 return {...receipt,name:parts.find(part=>part.name)?.name??receipt.path??receipt.identity??"文件",range:!whole&&merged.length?` (line ${merged.map(range=>`${range.start} to ${range.end}`).join(", ")})`:""};
}
