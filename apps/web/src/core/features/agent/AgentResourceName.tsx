import {ControlHint} from "../../components/ControlHint";
import {SourceHintContent,useSourceLinkInfo} from "./SourceHintContent";
import type {SourceObject} from "./sourceDirectory";

export function sourceObjectName(object:SourceObject|undefined):string|undefined {
 switch(object?.kind){
  case "pdf":return object.attachment.displayName;
  case "paper":return object.item.title;
  case "note":return object.snapshot.page.title||"未命名笔记";
  case "manuscript":return object.manuscript.title;
  case "manuscriptFile":return object.file.relativePath;
  case "structuredObject":return object.resource.title;
  case "structuredCatalog":return "全局阅读模板库";
  case "directory":return object.title;
  case "systemDocument":return object.document.name;
  case "folder":return object.path;
  case "highlight":return "高亮";
  default:return;
 }
}
/** Use the same resolved source hint as links and annotations. Internal routing
 * identities stay in raw receipts, rather than becoming visible resource names. */
export function AgentResourceName({path,name,className}:{readonly path:string;readonly name?:string|undefined;readonly className?:string|undefined}) {
 const source=path.startsWith("source://");
 const tail=path.split("/").at(-1);
 const filename=tail&&/\.[a-z\d]{1,12}$/i.test(tail)?tail:undefined;
 const label=name&&!name.startsWith("source://")&&(!source||name!==tail||!!filename)?name:filename;
 const info=useSourceLinkInfo(source?path:undefined,label);
 const title=label??sourceObjectName(info.object)??(source?info.lines.includes("来源不可用")?"来源不可用":"资源":path);
 const hint=source?<SourceHintContent path={path} lines={info.lines} includeDeveloperPath={false}/>:path;
 return <ControlHint content={hint} tooltipClassName="tool-file-path-hint"><span className={className} tabIndex={0}>{title}</span></ControlHint>;
}
