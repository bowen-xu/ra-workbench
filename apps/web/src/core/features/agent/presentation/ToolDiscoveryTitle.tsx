import { AgentResourceName } from "../../../public/agentContext/AgentResourceName";
import { AgentResourcePath } from "../../../public/agentContext/AgentResourcePath";
import type {ToolActivityRow} from "./toolActivityModel";
import {toolDiscoveryDescription} from "./toolDiscoveryActivity";
export function ToolDiscoveryTitle({row}:{readonly row:ToolActivityRow}){
 const value=toolDiscoveryDescription(row);if(!value)return null;
 if(value.kind==="list"){
  const scoped=!!(value.scope||value.listCategory);
  return <>{value.label}{(scoped||value.filters.length>0)&&<>（{scoped&&<>范围 {value.scope && !value.scope.path ? <span>{value.scope.name??"来源不可用"}</span> : <AgentResourcePath path={value.scope?.path} childCategory={value.listCategory}/>}</>}{scoped&&value.filters.length>0&&"，"}{value.filters.join("，")}）</>}</>;
 }
 return <>{value.label}{value.query&&` ${value.query}`}{(value.scope||value.filters.length>0)&&<>（{value.scope&&(value.scope.path?<AgentResourceName path={value.scope.path} name={value.scope.name} className="tool-read-file-name"/>:<span className="tool-read-file-name">{value.scope.name??"来源不可用"}</span>)}{value.scope&&value.filters.length>0&&"，"}{value.filters.join("，")}）</>}</>;
}
