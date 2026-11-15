import {Shimmer} from "../../../public/ui";
import {ChevronRight} from "lucide-react";
import {useMemo,useRef} from "react";
import {useNestedScrollRegion} from "../../../public/layout";
import {useScrollEdges} from "../../../public/layout";
import { useAgentInlineData } from "../../../public/agentPresentation/useAgentInlineData";
import type {ToolActivityRow} from "./toolActivityModel";
import {toolCommandOutput,toolCompactTitle,toolExpandedTitle,toolPatchOutcome,toolShellInvocation} from "./toolActivityPresentation";
import {toolPatchDiffFiles} from "./toolPatchDiffModel";
import {ToolPatchDiff,ToolPatchTitle} from "./ToolPatchDiff";
import {ToolContentCard} from "./ToolContentCard";
import {ToolReadTitle} from "./ToolReadTitle";
import {ToolEditTitle} from "./ToolEditTitle";
import {ToolEditFailure} from "./ToolEditFailure";
import {useStructuredPatchDiffs} from "./useStructuredPatchDiffs";
import {ToolDiscoveryTitle} from "./ToolDiscoveryTitle";
import {toolDiscoveryDescription} from "./toolDiscoveryActivity";
import {toolReadDescription} from "./toolReadActivity";
import {ToolActivityIcon} from "./ToolActivityIcon";
import {useTranscriptDisclosure} from "./TranscriptDisclosureState";
export function ToolActivityHistoryRow({row,disclosureId}:{readonly row:ToolActivityRow;readonly disclosureId?:string|undefined}):React.JSX.Element {
 const [open,setOpen]=useTranscriptDisclosure(disclosureId ? `tool:${disclosureId}` : undefined);
 const details=useRef<HTMLDetailsElement>(null);
 const disclosureBeforeClick=useRef(false);
 const raw=useAgentInlineData(row.reads?row.reads.map(part=>({call:part.call,result:part.result})):{call:row.call,result:row.result},"tool-row-data",()=>{if(details.current)details.current.open=disclosureBeforeClick.current;});
 const shell=toolShellInvocation(row),output=shell?toolCommandOutput(row):undefined;
 const editOutcome=toolPatchOutcome(row);
 const displayStatus=editOutcome==="failed"?"error":editOutcome==="declined"?"declined":row.status;
 const preparedFiles=useMemo(()=>toolPatchDiffFiles(row),[row.call,row.result,row.status]);
 const files=useStructuredPatchDiffs(row,preparedFiles);
 const contentViewport=useRef<HTMLDivElement>(null);
 useNestedScrollRegion(contentViewport,!!shell||files.length>0);
 const edges=useScrollEdges(contentViewport,files,!!shell||files.length>0);
 const outputText=output?.text??row.result?.content??"";
 const exitSeparator=outputText.endsWith("\n\n")?"":outputText.endsWith("\n")?"\n":"\n\n";
 const shellText=`$ ${shell??""}`+(row.result?`\n\n${outputText}${output?.exitCode!==undefined?`${exitSeparator}exit code: ${output.exitCode}`:""}`:"");
 const patchTitle=files.length?<ToolPatchTitle files={files}/>:undefined;
 const readTitle=toolReadDescription(row)?<ToolReadTitle row={row}/>:undefined;
 const editTitle=editOutcome?<ToolEditTitle row={row} outcome={editOutcome}/>:undefined;
 const discoveryTitle=toolDiscoveryDescription(row)?<ToolDiscoveryTitle row={row}/>:undefined;
 const title=<><ToolActivityIcon row={row}/>
  <Shimmer active={row.status==="running"} className="writing-agent-activity-title"><span className="tool-row-compact">{patchTitle??editTitle??readTitle??discoveryTitle??toolCompactTitle(row)}</span><span className="tool-row-expanded">{patchTitle??editTitle??readTitle??discoveryTitle??toolExpandedTitle(row)}</span></Shimmer></>;
 if(!shell&&!files.length)return <div className="tool-history-entry"><div className={`writing-agent-activity tool-history-row ${row.status}`} data-tool-status={row.status}>
  <div className="tool-history-static-summary">{title}{raw.trigger&&<span className="tool-row-data-slot">{raw.trigger}</span>}</div></div>
  {raw.content&&<div className="tool-history-content tool-history-raw-content">{raw.content}</div>}</div>;
 return <div className="tool-history-entry"><details ref={details} open={open} onToggle={event=>setOpen(event.currentTarget.open)} className={`writing-agent-activity tool-history-row ${displayStatus}`} data-tool-status={displayStatus} data-edit-status={editOutcome}>
  <summary aria-label={`${editOutcome?{completed:"已编辑",failed:"无法编辑",declined:"拒绝编辑"}[editOutcome]:row.status==="error"?"失败":row.status==="success"?"成功":"未完成"} · ${row.label}`}
   onClickCapture={event=>{if(event.target instanceof Element&&event.target.closest(".tool-row-data"))disclosureBeforeClick.current=details.current?.open??false;}}>
   {title}
   <ChevronRight size={14} aria-hidden="true" className="writing-agent-activity-chevron"/>
   {raw.trigger&&<span className="tool-row-data-slot">{raw.trigger}</span>}
  </summary>
  <div ref={contentViewport} className={`tool-history-content tool-row-content-viewport${files.length?" tool-patch-content":""}`} data-overflow-top={edges.top} data-overflow-bottom={edges.bottom} onScroll={edges.onScroll}>
   {shell&&(editOutcome==="failed"||editOutcome==="declined"?<ToolEditFailure row={row}/>:<ToolContentCard title="Shell" copy={shellText} label="复制命令与输出" className="tool-shell-card"><pre className="tool-shell-content"><span aria-label="调用参数">$ {shell}</span>{row.result&&<>{"\n\n"}<span aria-label="调用结果">{outputText}</span>{output?.exitCode!==undefined&&<>{exitSeparator}<span className="tool-shell-exit-code">exit code: {output.exitCode}</span></>}</>}</pre></ToolContentCard>)}
   {files.map((file,index)=><ToolPatchDiff key={`${file.path}:${index}`} file={file}/>)}
  </div>
 </details>{raw.content&&<div className="tool-history-content tool-history-raw-content">{raw.content}</div>}</div>;
}
