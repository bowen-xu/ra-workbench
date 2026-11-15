import {useEffect,useMemo,useState} from "react";
import {getStructuredResourceGateway} from "../../../public/structuredResources";
import type {ToolActivityRow} from "./toolActivityModel";
import {toolPatchApplied} from "./toolActivityPresentation";
import {toolPatchDiffFiles,type PatchDiffFile} from "./toolPatchDiffModel";
function object(value:unknown):Record<string,unknown>|undefined{return value&&typeof value==="object"&&!Array.isArray(value)?value as Record<string,unknown>:undefined;}
function result(row:ToolActivityRow){try{return object(JSON.parse(row.result?.content??""));}catch{return;}}
/** Legacy receipts compared a repair draft. Resolve only the exact historical
 * saved versions; the original receipt and the current resource are untouched. */
export function useStructuredPatchDiffs(row:ToolActivityRow,files:readonly PatchDiffFile[]):readonly PatchDiffFile[]{
 const targets=useMemo(()=>{
  if(!toolPatchApplied(row))return [];
  const value=result(row),previews=Array.isArray(value?.fileDiffs)?value.fileDiffs.map(object):[];
  return (Array.isArray(value?.targets)?value.targets:[]).flatMap(value=>{
   const target=object(value);
   let resource=object(target?.structuredResource);
   if(!resource&&typeof target?.message==="string"){try{resource=object(JSON.parse(target.message));}catch{/* Plain diagnostics have no saved resource identity. */}}
   if(typeof target?.path!=="string"||typeof resource?.resourceId!=="string"||typeof resource.revision!=="number"||!Number.isSafeInteger(resource.revision)||resource.revision<1||previews.some(preview=>preview?.path===target.path&&preview?.comparison==="saved"))return [];
   return [{path:target.path,resourceId:resource.resourceId,revision:resource.revision}];
  });
 },[row.call?.name,row.result?.content,row.status]);
 const key=JSON.stringify(targets);
 const [resolved,setResolved]=useState<{key:string;previews:readonly Record<string,unknown>[]} >();
 useEffect(()=>{
  if(!targets.length)return;
  let active=true;
  void Promise.all(targets.map(async target=>{
   try{
    const gateway=getStructuredResourceGateway();
    const [after,before]=await Promise.all([gateway.getVersion(target.resourceId,target.revision),target.revision===1?Promise.resolve(null):gateway.getVersion(target.resourceId,target.revision-1)]);
    if(after.id!==target.resourceId||after.revision!==target.revision||before&&(before.id!==target.resourceId||before.revision!==target.revision-1))throw new Error("历史修改快照身份不匹配");
    const preview={path:target.path,comparison:"saved",beforeSaved:before?.content??null,afterSaved:after.content};
    const rows=[preview.beforeSaved,preview.afterSaved].reduce<number>((total,content)=>total+(content==null?0:JSON.stringify(content,null,2).split("\n").length),0);
    return JSON.stringify(preview).length>48_000||rows>600?{path:target.path,comparison:"saved",omitted:true}:preview;
   }catch(reason){
    const error=object(reason),code=typeof error?.code==="string"?error.code:"snapshot_unavailable";
    const cause=reason instanceof Error?reason.message:typeof error?.message==="string"?error.message:"历史快照读取失败";
    return {path:target.path,comparison:"saved",omitted:true,omissionReason:`无法读取当次保存前后快照（${code}）：${cause}`};
   }
  })).then(previews=>{if(active)setResolved({key,previews});});
  return()=>{active=false;};
 },[key,targets]);
 if(!targets.length)return files;
 if(resolved?.key!==key)return files.map(file=>targets.some(target=>target.path===file.path)?{...file,lines:[],omitted:true,omissionReason:"正在读取当次正式保存前后快照…"}:file);
 const value=result(row),previews=Array.isArray(value?.fileDiffs)?value.fileDiffs:[];
 return toolPatchDiffFiles({...row,result:{...row.result!,content:JSON.stringify({...value,fileDiffs:[...previews.filter(value=>!targets.some(target=>object(value)?.path===target.path)),...resolved.previews]})}});
}
