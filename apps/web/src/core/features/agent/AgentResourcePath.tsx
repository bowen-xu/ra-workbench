import {formatSourceLink,parseSourceLink} from "@research-assistant/core/source";
import {ControlHint} from "../../components/ControlHint";
import {sourceObjectName} from "./AgentResourceName";
import {SourceHintContent,useSourceLinkInfos} from "./SourceHintContent";
import type {SourceObject} from "./sourceDirectory";
function category(object:SourceObject|undefined):string|undefined {
 switch(object?.kind){
  case "paper":case "pdf":return "所有文献";
  case "note":return "所有笔记";
  case "manuscript":case "folder":case "manuscriptFile":return "所有写作项目";
  default:return;
 }
}
/** Display-only hierarchy: IDs still route through the canonical source path.
 * Each segment resolves under its actual parent, never by scanning search hits. */
export function AgentResourcePath({path,childCategory}:{readonly path?:string|undefined;readonly childCategory?:string|undefined}) {
 let prefixes:string[]=[];
 if(path){try{const link=parseSourceLink(path);prefixes=link.path.map((_,index)=>formatSourceLink({path:link.path.slice(0,index+1)}));}catch{/* Preserve the raw address in the original receipt only. */}}
 const infos=useSourceLinkInfos(prefixes);
 const nodes=prefixes.map(prefix=>infos.get(prefix));
 const root=nodes[0]?.object?.kind === "directory" ? undefined : category(nodes[0]?.object);
 const names=nodes.map(info=>{
  const object=info?.object;
  const name=object?.kind==="folder"?object.path.split("/").at(-1):object?.kind==="manuscriptFile"?object.file.relativePath.split("/").at(-1):sourceObjectName(object);
  return name??(info?"来源不可用":"正在读取来源…");
 });
 const segments=[...root?[root]:[],...path?names.length?names:["来源不可用"]:[],...childCategory?[childCategory]:[]];
 const label=segments.join(" / ");
 const leaf=nodes.at(-1);
 const hint=path?<SourceHintContent path={path} lines={[label,...(leaf?.lines??[]).filter(line=>line!==label)]} includeDeveloperPath={false}/>:label;
 return <ControlHint content={hint} tooltipClassName="tool-file-path-hint"><span className="tool-read-file-name" tabIndex={0}>{label}</span></ControlHint>;
}
