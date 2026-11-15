import {Fragment,useMemo} from "react";
import { AgentResourceName } from "../../../public/agentContext/AgentResourceName";
import {codeHighlightTokens} from "../../../public/codeEditorExtensions";
import {ToolContentCard} from "./ToolContentCard";
import type {PatchDiffFile} from "./toolPatchDiffModel";

function FileName({file}:{readonly file:PatchDiffFile}) {
 return <AgentResourceName path={file.path} name={file.name} className="tool-patch-file-name"/>;
}
function Counts({file}:{readonly file:PatchDiffFile}) {return file.omitted||!file.lines.length?null:<span className="tool-patch-counts"><span className="tool-patch-added-count">+{file.additions}</span><span className="tool-patch-deleted-count">-{file.deletions}</span></span>;}
export function ToolPatchTitle({files}:{readonly files:readonly PatchDiffFile[]}) {
 const first=files[0];if(!first)return null;
 return <>已编辑 {files.length===1?<><FileName file={first}/> <Counts file={first}/></>:`${files.length} 个文件`}</>;
}
function language(name:string):string {const ext=name.split(".").at(-1)?.toLowerCase();return ({ts:"typescript",tsx:"typescript",js:"javascript",jsx:"javascript",py:"python",json:"json",tex:"latex",sty:"latex"} as Record<string,string>)[ext??""]??"text";}
export function ToolPatchDiff({file}:{readonly file:PatchDiffFile}) {
 const source=file.lines.map(line=>line.text).join("\n");
 const tokens=useMemo(()=>codeHighlightTokens(source,language(file.name)),[source,file.name]);
 let offset=0;
 return <ToolContentCard title={<span className="tool-patch-file-title"><FileName file={file}/><Counts file={file}/></span>} copy={file.patch} label="复制文件补丁" className="tool-patch-diff">
  {file.omitted?<p className="tool-patch-unavailable">{file.omissionReason??"差异预览超出记录预算，完整补丁可通过复制按钮查看。"}</p>:!file.lines.length?<p className="tool-patch-unavailable">这条历史记录没有保存差异正文。</p>:<div className="tool-patch-lines" aria-label={`${file.name} 文件差异`}>
   {file.lines.map((line,index)=>{const start=offset,end=start+line.text.length;offset=end+1;
    if(line.kind==="gap")return <div key={index} className="tool-patch-line gap" role="separator" aria-label="不连续的编辑片段"/>;
    let cursor=start;
    const spans=tokens.filter(token=>token.to>start&&token.from<end).map((token,i)=>{const from=Math.max(start,token.from),to=Math.min(end,token.to);const preceding=source.slice(cursor,from);cursor=to;return <Fragment key={i}>{preceding}<span className={token.classes}>{source.slice(from,to)}</span></Fragment>;});
    return <div key={index} className={`tool-patch-line ${line.kind}`}><span className="tool-patch-line-number" aria-hidden="true">{line.kind==="deleted"?line.oldLine:line.newLine}</span><span className="tool-patch-line-marker" aria-hidden="true">{line.kind==="added"?"+":line.kind==="deleted"?"−":""}</span><code>{spans}{source.slice(cursor,end)}{"\n"}</code></div>;
   })}</div>}
 </ToolContentCard>;
}
