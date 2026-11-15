import { presentableDiff } from "@codemirror/merge";
import { Fragment, useMemo, type MouseEvent, type ReactNode } from "react";
import { DiffHighlight, EditorDiffHighlight, type EditorDiffHighlightAppearance } from "../editor-diff/EditorDiffHighlight";
/** Reuse the bounded text matcher; cards remain ordinary selectable prose. */
export function StructuredTextDiff({before,after,incoming,appearance,onEditAfter,renderAfter}:{readonly before:string;readonly after:string;readonly incoming?:string;readonly appearance?:EditorDiffHighlightAppearance;
  readonly onEditAfter?:(event:MouseEvent<HTMLElement>)=>void;
  readonly renderAfter?:(content:ReactNode)=>ReactNode;
}){
  const changes=useMemo(()=>presentableDiff(before,after,{scanLimit:1000,timeout:50}),[before,after]);
  function side(text:string,removed:boolean){
    let start=0;
    const spans=changes.map((change,index)=>{
      const from=removed?change.fromA:change.fromB,to=removed?change.toA:change.toB;
      const common=text.slice(start,from);start=to;
      const changed=text.slice(from,to);
      return <Fragment key={index}>{common}{changed&&<DiffHighlight kind={removed?"removed":"added"}>{changed}</DiffHighlight>}</Fragment>;
    });
    return <>{spans}{text.slice(start)}</>;
  }
  return <EditorDiffHighlight appearance={appearance}><div className="structured-card-diff" aria-label="正文修改差异">
    {(before!==after||incoming!==undefined)&&before&&<div className="structured-card-diff-before" aria-label="修改前">{side(before,true)}</div>}
    {incoming!==undefined&&<DiffHighlight kind="incoming" scope="block"><div aria-label="incoming">{incoming}</div></DiffHighlight>}
    {(before!==after||incoming!==undefined)&&(after||renderAfter)&&<div className="structured-card-diff-after" aria-label="修改后" onDoubleClick={onEditAfter}>{renderAfter?renderAfter(side(after,false)):side(after,false)}</div>}
    {before===after&&incoming===undefined&&(renderAfter?renderAfter(after):<p onDoubleClick={onEditAfter}>{after||"已读取材料中未找到"}</p>)}
    {before===after&&incoming===undefined&&<span className="structured-card-diff-note">字段依据或状态已修改</span>}
  </div></EditorDiffHighlight>;
}
