import {Fragment} from "react";
import { AgentResourceName } from "../../../public/agentContext/AgentResourceName";
import type {ToolActivityRow} from "./toolActivityModel";
import {toolPatchTargets} from "./toolActivityPresentation";
export function ToolEditTitle({row,outcome}:{readonly row:ToolActivityRow;readonly outcome:"completed"|"failed"|"declined"}){
 const targets=toolPatchTargets(row);
 return <>{({completed:"已编辑",failed:"无法编辑",declined:"拒绝编辑"})[outcome]} {targets.length?targets.map((target,index)=><Fragment key={`${target.path}:${index}`}>{index>0&&"、"}<AgentResourceName path={target.path} name={target.name} className="tool-patch-file-name"/></Fragment>):"文件"}</>;
}
