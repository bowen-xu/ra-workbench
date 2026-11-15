import {Fragment} from "react";
import { AgentResourceName } from "../../../public/agentContext/AgentResourceName";
import {ToolContentCard} from "./ToolContentCard";
import {toolPatchTargets} from "./toolActivityPresentation";
import {toolEditDiagnostics} from "./toolEditDiagnostics";
import type {ToolActivityRow} from "./toolActivityModel";
export function ToolEditFailure({row}:{readonly row:ToolActivityRow}){
 const targets=toolPatchTargets(row),lines=toolEditDiagnostics(row);
 const copy=`对象：${targets.map(target=>target.name??(target.path.startsWith("source://")?"资源":target.path)).join("、")||"文件"}\n${lines.join("\n")}`;
 return <ToolContentCard title="编辑结果" copy={copy} label="复制编辑结果" className="tool-shell-card">
  <div className="tool-shell-content"><div>对象：{targets.length?targets.map((target,index)=><Fragment key={`${target.path}:${index}`}>{index>0&&"、"}<AgentResourceName path={target.path} name={target.name} className="tool-read-file-name"/></Fragment>):"文件"}</div>
   <div aria-label="调用结果">{lines.map((line,index)=><div key={index}>{line}</div>)}</div>
  </div>
 </ToolContentCard>;
}
