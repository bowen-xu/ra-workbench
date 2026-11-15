import type { DocumentPath, DocumentReviewFragment } from "@research-assistant/core/hbs";
import { structuredResourceDiffFields } from "@research-assistant/core/structured-resources";

export function structuredTextLeaves(value: unknown, path: DocumentPath = []): {path:DocumentPath;text:string}[] {
  if (Array.isArray(value)) return value.flatMap((item,index) => {
    const identity=item && typeof item === "object" ? item as {id?:string;field?:string}:{};
    return structuredTextLeaves(item,[...path,identity.id?{key:"id",value:identity.id}:identity.field?{key:"field",value:identity.field}:index]);
  });
  if (!value || typeof value !== "object") return [];
  return Object.entries(value).flatMap(([key,item]) => ["text","question","description"].includes(key) && typeof item === "string"
    ? [{path:[...path,key],text:item}] : typeof item === "object" ? structuredTextLeaves(item,[...path,key]):[]);
}
export function containsStructuredPath(parent:DocumentPath,child:DocumentPath):boolean {
  return parent.length<=child.length && parent.every((part,index)=>JSON.stringify(part)===JSON.stringify(child[index]));
}
export function sameStructuredPath(left:DocumentPath,right:DocumentPath):boolean {return left.length===right.length&&containsStructuredPath(left,right);}
function keyedPaths(value:unknown,path:DocumentPath=[]):{path:DocumentPath;value:unknown}[] {
  if(Array.isArray(value))return value.flatMap(item=>{
    if(!item||typeof item!=="object")return [];
    const identity=item as {id?:string;field?:string};
    const key=typeof identity.id==="string"?{key:"id",value:identity.id}:typeof identity.field==="string"?{key:"field",value:identity.field}:undefined;
    return key?[{path:[...path,key],value:item},...keyedPaths(item,[...path,key])]:[];
  });
  return value&&typeof value==="object"?Object.entries(value).flatMap(([key,item])=>keyedPaths(item,[...path,key])):[];
}
export type StructuredField = ReturnType<typeof structuredResourceDiffFields>[number];
export interface StructuredFieldUnit {readonly field:StructuredField;readonly path:DocumentPath}
export function structuredFieldUnits(value:unknown):StructuredFieldUnit[] {
  const paths=keyedPaths(value);
  return structuredResourceDiffFields(value).filter(field=>!field.id.startsWith("$")||field.id.startsWith("$retained:")).flatMap(field=>{
    const entry=paths.find(entry=>entry.value===field.value);return entry?[{field,path:entry.path}]:[];
  });
}
export interface StructuredPlacedFragment {readonly decisionId:string;readonly fragment:DocumentReviewFragment;readonly field:StructuredFieldUnit;readonly key:string}
export function structuredOldSuccessor(item:StructuredPlacedFragment,siblings:readonly StructuredPlacedFragment[]):DocumentPath|undefined {
  const path=item.fragment.beforePath,identity=path?.at(-1),order=item.fragment.placement?.before;
  if(!path||typeof identity!=="object"||!order)return item.fragment.path;
  const moving=new Set(siblings.filter(sibling=>sibling.fragment.positionChanged&&sibling.fragment.beforePath
    &&sameStructuredPath(sibling.fragment.beforePath.slice(0,-1),path.slice(0,-1)))
    .map(sibling=>sibling.fragment.path?.at(-1)).filter(part=>typeof part==="object").map(part=>part.value));
  for(const next of order.slice(order.indexOf(identity.value)+1))if(!moving.has(next)&&item.fragment.placement?.after.includes(next))return [...path.slice(0,-1),{key:identity.key,value:next}];
  return undefined;
}
