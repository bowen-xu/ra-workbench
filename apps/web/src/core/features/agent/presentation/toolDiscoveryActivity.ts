import {formatSourceLink} from "@research-assistant/core/source";
import type {ToolActivityRow} from "./toolActivityModel";
function record(text:string|undefined):Record<string,unknown>{try{const value:unknown=JSON.parse(text??"");return value&&typeof value==="object"&&!Array.isArray(value)?value as Record<string,unknown>:{};}catch{return {};}}
function text(value:unknown):string|undefined{return typeof value==="string"&&value.length>0?value:undefined;}
/** Scope comes from explicit arguments, never from the subset of search hits. */
export function toolDiscoveryDescription(row:ToolActivityRow){
 const tool=row.call?.name??row.result?.name??"";
 const kind=["research_search_sources","search_sources"].includes(tool)?"search":tool==="research_find_sources"?"find":tool==="research_list_sources"?"list":undefined;
 if(!kind)return;
 const args=record(row.call?.content),result=record(row.result?.content);
 const query=text(args.query);
 if(kind==="search"&&!query)return;
 const labels={search:{success:"已搜索",running:"正在搜索",error:"搜索失败",interrupted:"搜索中断"},find:{success:"已查找",running:"正在查找",error:"查找失败",interrupted:"查找中断"},list:{success:"已列出文件",running:"正在列出文件",error:"列出文件失败",interrupted:"列出文件中断"}};
 const type=text(args.resourceType);
 const listCategory=kind==="list"&&type?({note:"所有笔记",paper:"所有文献",manuscriptFile:"所有写作项目",paperMetadata:"所有文献 / 元数据",paperPortrait:"所有文献 / 文献画像",portrait:"所有文献 / 文献画像",structuredReading:"所有文献 / 结构化阅读",readingTemplate:"全局阅读模板库",templateCatalog:"全局阅读模板库"} as Record<string,string>)[type]:undefined;
 const filters=([["nameQuery","名称"],["pathGlob","路径"],["resourceType","类型"]] as const).flatMap(([key,label])=>text(args[key])&&!(key==="resourceType"&&listCategory)?[`${label} ${args[key]}`]:[]);
 const identity=text(args.fileId)??text(args.sourceId)??text(args.resourceId);
 let scope: {path?:string;name?:string}|undefined;
 if(identity){
  const metadata=result.scope&&typeof result.scope==="object"?result.scope as Record<string,unknown>:{};
  // Exact frozen metadata also handles a zero-match file search. Older receipts
  // may resolve object IDs through the same source directory as read titles.
  let path=text(metadata.sourceUrl);
  if(!path && identity.startsWith("source://")) path=identity;
  if(!path && type === "note") path=formatSourceLink({path:["notes",identity]});
  if(!path && type === "paperMetadata") path=formatSourceLink({path:["library",identity]});
  if(!path && type === "templateCatalog") path="source://agents/skills/edit-structured-resources/assets/reading";

  const name=text(metadata.name);
  scope={...path?{path}:{},...name?{name}:{}};
 }
 const scopedCategory=scope&&type?({note:"笔记",paper:"PDF",manuscriptFile:"文件",paperMetadata:"元数据",paperPortrait:"文献画像",portrait:"文献画像",structuredReading:"结构化阅读",readingTemplate:"阅读模板",templateCatalog:"全局阅读模板库"} as Record<string,string>)[type]:undefined;
 return {kind,label:labels[kind][row.status],query,filters,scope,listCategory:scopedCategory??listCategory};
}
export function toolDiscoveryText(row:ToolActivityRow):string|undefined {
 const value=toolDiscoveryDescription(row);if(!value)return;
 const range=value.kind==="list"?[...(value.scope?[value.scope.name??"资源"]:[]),...(value.listCategory?[value.listCategory]:[])].join(" / "):undefined;
 const annotations=[...(value.kind==="list"?range?[`范围 ${range}`]:[]:value.scope?[value.scope.name??"资源"]:[]),...value.filters];
 return `${value.label}${value.query?` ${value.query}`:""}${annotations.length?`（${annotations.join("，")}）`:""}`;
}
