import type {ToolActivityRow} from "./toolActivityModel";
import {toolPatch,toolPatchApplied} from "./toolActivityPresentation";
import {writingLineDiff} from "./writingChangeStats";

export interface PatchDiffLine { readonly kind:"context"|"added"|"deleted"|"gap"; readonly text:string; readonly oldLine?:number; readonly newLine?:number }
export interface PatchDiffFile { readonly path:string; readonly name:string; readonly lines:readonly PatchDiffLine[]; readonly additions:number; readonly deletions:number; readonly patch:string; readonly omitted:boolean; readonly omissionReason?:string }
function savedRows(before:unknown,after:unknown):PatchDiffLine[]{
 const text=(value:unknown)=>value==null?"":JSON.stringify(value,null,2);
 const value=writingLineDiff(text(before),text(after)),lines:PatchDiffLine[]=[];
 let a=0,b=0;
 for(const change of value.changes){
  while(a<change.fromA){lines.push({kind:"context",text:value.before[a]!,oldLine:++a,newLine:++b});}
  while(a<change.toA)lines.push({kind:"deleted",text:value.before[a]!,oldLine:++a});
  while(b<change.toB)lines.push({kind:"added",text:value.after[b]!,newLine:++b});
 }
 while(b<value.after.length){lines.push({kind:"context",text:value.after[b]!,oldLine:++a,newLine:++b});}
 return lines;
}
function object(value:unknown):Record<string,unknown>|undefined {return value&&typeof value==="object"&&!Array.isArray(value)?value as Record<string,unknown>:undefined;}
function json(text:string|undefined):Record<string,unknown>|undefined {try{return object(JSON.parse(text??""));}catch{return undefined;}}
function basename(path:string):string {const name=path.split(/[\\/]/u).at(-1)??path;if(!path.startsWith("source://"))return name;try{return decodeURIComponent(name);}catch{return name;}}
function rows(lines:readonly string[],oldStart?:number,newStart?:number):PatchDiffLine[] {
 let old=oldStart,next=newStart;
 return lines.flatMap(line=>{
  const marker=line[0],text=line.slice(1);
  if(marker!=="+"&&marker!=="-"&&marker!==" ")return [];
  const value:PatchDiffLine={kind:marker==="+"?"added":marker==="-"?"deleted":"context",text,
   ...(marker!=="+"&&old!==undefined?{oldLine:old}:{}),...(marker!=="-"&&next!==undefined?{newLine:next}:{})};
  if(marker!=="+"&&old!==undefined)old++;
  if(marker!=="-"&&next!==undefined)next++;
  return [value];
 });
}
/** Presentation-only fallback for historical calls without execution positions. */
function submittedFiles(patch:string):{path:string;patch:string;lines:PatchDiffLine[]}[] {
 const files:{path:string;patch:string;lines:PatchDiffLine[]}[]=[];
 let current:typeof files[number]|undefined,body:string[]=[],old:number|undefined,next:number|undefined;
 function flush(){if(current){current.lines.push(...rows(body,old,next));body=[];}}
 for(const line of patch.split("\n")) {
  const header=/^\*\*\* (Update|Add|Delete) File: (.+)$/u.exec(line);
  if(header){flush();current={path:header[2]!,patch:line,lines:[]};files.push(current);old=undefined;next=header[1]==="Add"?1:undefined;continue;}
  if(!current||line==="*** End Patch"||line==="*** Begin Patch")continue;
  current.patch+="\n"+line;
  if(line.startsWith("@@")){flush();if(current.lines.length)current.lines.push({kind:"gap",text:"…"});const at=/^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/u.exec(line);old=at?Number(at[1]):undefined;next=at?Number(at[2]):undefined;}
  else body.push(line);
 }
 flush();return files;
}
export function toolPatchDiffFiles(row:ToolActivityRow):PatchDiffFile[] {
 if(!toolPatchApplied(row))return [];
 const result=json(row.result?.content),submitted=submittedFiles(toolPatch(row)??"");
 const previews=Array.isArray(result?.fileDiffs)?result.fileDiffs.map(object).filter(value=>value!==undefined):[];
 return submitted.map(file=>{
  const preview=previews.find(value=>value.path===file.path);
  const target=Array.isArray(result?.targets)?result.targets.map(object).find(value=>value?.path===file.path):undefined;
  const lines:PatchDiffLine[]=[];
  if(Array.isArray(preview?.hunks))for(const value of preview.hunks){const hunk=object(value);
   if(!hunk||!Array.isArray(hunk.lines)||!hunk.lines.every(line=>typeof line==="string"))continue;
   if(lines.length)lines.push({kind:"gap",text:"…"});
   const start=(value:unknown)=>typeof value==="number"&&Number.isSafeInteger(value)&&value>=1?value:undefined;
   lines.push(...rows(hunk.lines,start(hunk.oldStart),start(hunk.newStart)));
  }
  const saved=preview?.comparison==="saved";
  const frozen=saved&&"beforeSaved" in preview&&"afterSaved" in preview?savedRows(preview.beforeSaved,preview.afterSaved):undefined;
  const proposed=frozen??(preview?lines:file.lines);
  const omitted=preview?.omitted===true||proposed.length>600||proposed.reduce((size,line)=>size+line.text.length,0)>48_000;
  const shown=omitted?[]:proposed;
  const name=[target?.name,preview?.name].find((value):value is string=>typeof value==="string"&&value.length>0)??basename(file.path);
  return {path:file.path,name,lines:shown,
   additions:shown.filter(line=>line.kind==="added").length,deletions:shown.filter(line=>line.kind==="deleted").length,
   patch:`*** Begin Patch\n${file.patch}\n*** End Patch`,omitted,...typeof preview?.omissionReason==="string"?{omissionReason:preview.omissionReason}:{}};
 });
}
