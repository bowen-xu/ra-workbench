import { Fragment, useEffect, useMemo, useRef, useState, useSyncExternalStore, type MouseEvent, type ReactNode } from "react";
import { EditorView } from "@codemirror/view";
import type { EditorState } from "@codemirror/state";
import { undo, redo } from "@codemirror/commands";
import { Pencil } from "lucide-react";
import { projectDocumentReview, type DocumentChange, type DocumentPath, type DocumentReviewProjector, type HbsSession } from "@research-assistant/core/hbs";
import { replaceStructuredFieldText, structuredResourceDiffFields, type StructuredResourceKind } from "@research-assistant/core/structured-resources";
import { StructuredFieldCard } from "./StructuredFieldCard";
import { StructuredTextDiff } from "./StructuredTextDiff";
import { StructuredReviewControls } from "./StructuredReviewControls";
import { EditorDiffChunkActions } from "../editor-diff/EditorDiffChunkButtons";
import { EditorReviewLocationActions } from "../editor-diff/EditorReviewLocationActions";
import type { EditorReviewAction, EditorReviewConfiguration } from "../editor-diff/EditorReviewActions";
import { ControlHint } from "../ControlHint";
import { HbsLeafEditor } from "./HbsLeafEditor";
import { StructuredValueView } from "./StructuredValueView";
import { StructuredFieldTextEditor, structuredFieldCaretOffset } from "./StructuredFieldTextEditor";
import { projectStructuredFieldInput } from "./structuredFieldInput";
import { DiffHighlight, EditorDiffHighlight } from "../editor-diff/EditorDiffHighlight";
import { containsStructuredPath as contains, sameStructuredPath as same, structuredFieldUnits, structuredOldSuccessor,
  structuredTextLeaves, type StructuredField, type StructuredPlacedFragment } from "./structuredReviewUnits";
import "./StructuredResourceReview.css";
import "./HbsDocumentEditor.css";

// The weak key is the live domain owner. A host handoff retains native CM
// history without extending the resource lifetime or retaining old listeners.
const retainedHistory=new WeakMap<HbsSession<unknown>,EditorState>();

interface EditingField {
  readonly id:string;readonly text:string;readonly source:unknown;readonly draft:unknown;readonly draftBaseline:unknown;
  readonly initialCaret?:number;
}
export interface StructuredNativeHistoryAction {readonly undo:()=>boolean;readonly redo:()=>boolean}
export interface HbsStructuredEditorProps {
  readonly session:HbsSession<unknown>;
  readonly configuration?:EditorReviewConfiguration;
  readonly reviewProjector?:DocumentReviewProjector;
  readonly resourceKind?:StructuredResourceKind;
  readonly title?:string;
  readonly resourceId?:string;
  readonly busy?:boolean;
  readonly submissionBlocked?:string;
  readonly renderIndicators?:(field:StructuredField)=>ReactNode;
  readonly renderFooter?:(field:StructuredField)=>ReactNode;
  readonly onKeepUnreviewed?:()=>void;
  readonly onHistoryAction?:(action:StructuredNativeHistoryAction)=>void;
  readonly onNativeHistoryReady?:(action:StructuredNativeHistoryAction)=>()=>void;
  readonly onHistoryCommand?:(direction:"undo"|"redo")=>void;
  readonly panelOwnsHistory?:boolean;
}
function display(field:StructuredField,kind:StructuredResourceKind):string {
  if(kind==="readingTemplate")return `${field.title}\n${field.text}${field.color?`\n颜色：${field.color}`:""}`;
  const value=field.value as {label?:string};
  return kind==="portrait"&&value.label?`${value.label}\n${field.text}`:field.text;
}
function SnapshotContent({value}:{readonly value:unknown}):React.JSX.Element {
  const leaves=structuredTextLeaves(value);
  return <>{leaves.length?leaves.map(leaf=><p key={JSON.stringify(leaf.path)}>{leaf.text}</p>):<StructuredValueView value={value}/>}</>;
}
/** One production card/editor tree for the App and all HBS harnesses. The host
 * supplies resource metadata and persistence; cards retain their established
 * inline card interaction instead of exposing a file editor. */
export function HbsStructuredEditor({session,configuration,reviewProjector=projectDocumentReview,resourceKind="structuredReading",
  title="H 结构化资源",resourceId,busy=false,submissionBlocked,renderIndicators,renderFooter,onKeepUnreviewed,onHistoryAction,onNativeHistoryReady,onHistoryCommand,panelOwnsHistory=false}:HbsStructuredEditorProps):React.JSX.Element {
  const state=useSyncExternalStore(session.subscribe,session.getSnapshot);
  const surface=useRef<HTMLElement>(null),historyHost=useRef<HTMLDivElement>(null);
  const runner=useRef<((action:()=>void)=>void)|undefined>(undefined);
  const [editing,setEditing]=useState<EditingField>(),[saving,setSaving]=useState(false),[activeId,setActiveId]=useState("");
  const historyActions=useMemo(()=>({bind:(run:(action:()=>void)=>void)=>{runner.current=run;return()=>{runner.current=undefined;};}}),[]);
  const stateRetention=useMemo(()=>({load:()=>retainedHistory.get(session),save:(value:EditorState)=>{retainedHistory.set(session,value);}}),[session]);
  const nativeHistory=():EditorView|undefined=>{
    const host=historyHost.current?.querySelector<HTMLElement>(".cm-editor");return host?EditorView.findFromDOM(host)??undefined:undefined;
  };
  const nativeActions=useMemo(()=>{
    const replay=(command:typeof undo)=>{
      const editor=nativeHistory(),previous=session.getSnapshot();if(!editor||!command(editor))return false;
      const shown=session.getSnapshot();
      setEditing(current=>{
        if(!current)return current;
        // Apply only the replayed delta to the private input draft. Copying
        // the full shown document would copy compatible external updates into
        // the private draft rather than retaining the input's own delta.
        const draft=session.adapter.merge(previous.content,shown.content,current.draft).value;
        const draftBaseline=session.adapter.merge(previous.baselineProjection,shown.baselineProjection,current.draftBaseline).value;
        const field=structuredResourceDiffFields(draft).find(field=>field.id===current.id);
        return field?{...current,text:field.text,draft,draftBaseline}:undefined;
      });
      return true;
    };
    return {undo:()=>replay(undo),redo:()=>replay(redo)};
  },[session]);
  useEffect(()=>onNativeHistoryReady?.(nativeActions),[onNativeHistoryReady,nativeActions]);
  const perform=(action:()=>void):boolean=>{
    if(!runner.current){session.record("MODEL_HISTORY_UNAVAILABLE：结构审阅历史端口尚未就绪，输入与变更已保留。");return false;}
    runner.current(action);
    onHistoryAction?.(nativeActions);
    return true;
  };
  const decide=(ids:readonly string[],action:EditorReviewAction)=>{
    if(submissionBlocked){session.record(submissionBlocked);return false;}
    if(disabled)return false;
    // Input already belongs to H. Review that current value in one existing
    // HBS operation; a preceding blur-save must not swallow or stale the click.
    const performed=perform(()=>session.resolveMany(ids,action));
    if(performed)setEditing(undefined);
    return performed;
  };
  const disabled=busy||saving;
  const isReviewTarget=(target:EventTarget|null|undefined)=>target instanceof Element&&(
    !!surface.current?.contains(target)&&!!target.closest(".structured-resource-bulk,.writing-diff-chunk-actions")
    ||!!target.closest(".structured-review-menu"));
  const fields=structuredFieldUnits(state.content),beforeFields=structuredFieldUnits(state.baselineProjection);
  const changes=[...state.conflicts,...state.changes.filter(change=>!state.conflicts.some(conflict=>conflict.id===change.id))];
  const decisions=changes.map(change=>{
    const conflict=state.conflicts.find(item=>item.id===change.id);
    const projection=conflict?{decisionId:change.id,fragments:[{beforePath:change.beforePath,path:change.path,before:change.before,after:change.after,positionChanged:false,contentChanged:true}],fallbackReason:change.reason??"冲突按完整授权单元核对"}:reviewProjector(change);
    return {change,conflict,projection};
  });
  const placed:StructuredPlacedFragment[]=decisions.flatMap(({change,conflict,projection})=>conflict?[]:projection.fragments.flatMap((fragment,index)=>{
    const path=fragment.path??fragment.beforePath,field=path&&[...fields,...beforeFields].find(unit=>contains(unit.path,path));
    const wholeField=field&&same(field.path,path??[]);
    return field&&(wholeField||fragment.positionChanged)?[{decisionId:change.id,fragment,field,key:`${change.id}:${index}`}]:[];
  }));
  const sourceOrder=(item:StructuredPlacedFragment)=>item.fragment.beforePath?item.fragment.placement?.before.indexOf(String((item.fragment.beforePath.at(-1) as {value?:string})?.value))??0:0;
  const oldItems=placed.filter(item=>item.fragment.beforePath).sort((a,b)=>sourceOrder(a)-sourceOrder(b));
  const fieldOld=oldItems.filter(item=>same(item.field.path,item.fragment.beforePath??[]));
  const nestedOld=oldItems.filter(item=>!same(item.field.path,item.fragment.beforePath??[]));
  const positionOnly=(change:DocumentChange)=>{const fragments=reviewProjector(change).fragments;return fragments.length>0&&fragments.every(item=>item.positionChanged&&!item.contentChanged);};
  const forField=(path:DocumentPath)=>changes.filter(change=>contains(path,change.path)&&!positionOnly(change));
  const jump=(key:string,side:"before"|"after")=>{
    const target=[...surface.current?.querySelectorAll<HTMLElement>("[data-hbs-review-pair]")??[]].find(element=>element.dataset.hbsReviewPair===key&&element.dataset.reviewLocationTarget===side);
    target?.scrollIntoView({block:"nearest",inline:"nearest",behavior:"auto"});target?.querySelector<HTMLButtonElement>("[data-review-location-jump]")?.focus({preventScroll:true});
  };
  const locationActions=(item:StructuredPlacedFragment,side:"before"|"after")=>item.fragment.positionChanged?<div key={item.key} data-hbs-review-pair={item.key} data-review-location-target={side}>
    <EditorReviewLocationActions side={side} configuration={configuration} disabled={disabled} onJump={()=>jump(item.key,side==="before"?"after":"before")} onResolve={action=>decide([item.decisionId],action)}/>
  </div>:null;
  const controls=(selected:readonly DocumentChange[])=>{
    if(!selected.length)return null;
    const conflict=selected.some(change=>state.conflicts.some(item=>item.id===change.id));
    const props={number:Math.max(1,changes.findIndex(change=>change.id===selected[0]?.id)+1),disabled,configuration};
    return conflict?<EditorDiffChunkActions {...props} conflict onResolve={action=>decide(selected.map(change=>change.id),action)}/>
      :<EditorDiffChunkActions {...props} onResolve={action=>decide(selected.map(change=>change.id),action)}/>;
  };
  const old=(item:StructuredPlacedFragment)=><div key={item.key} data-hbs-review-decision={item.decisionId} data-review-location="before" contentEditable={false}>
    <DiffHighlight kind="removed" scope="block">{same(item.field.path,item.fragment.beforePath??[])?<StructuredFieldCard id={item.field.field.id} title={item.field.field.title} pending appearance={resourceKind==="structuredReading"?"reading":"plain"} {...(item.field.field.color?{color:item.field.field.color}:{})}>
      <p>{display(item.field.field,resourceKind)||"已读取材料中未找到"}</p>
    </StructuredFieldCard>:<SnapshotContent value={item.fragment.before}/>}</DiffHighlight>
    {locationActions(item,"before")}{!item.fragment.path&&controls(changes.filter(change=>change.id===item.decisionId))}
  </div>;
  const startEditing=(field:StructuredField,event?:MouseEvent<HTMLElement>)=>{
    if(disabled||editing?.id===field.id)return;
    const initialCaret=event?structuredFieldCaretOffset(event):undefined;
    setEditing({id:field.id,text:field.text,source:state.content,draft:state.content,draftBaseline:state.baselineProjection,
      ...(initialCaret===undefined?{}:{initialCaret})});
  };
  const changeText=(text:string)=>{
    if(!editing)return;
    try{
      const draft=replaceStructuredFieldText(editing.source,editing.id,text);
      // A textarea edits the real field in the complete H document. Its local
      // delta is rebased over compatible incoming changes without saving S or
      // accepting any candidate. The same native history owns all decisions.
      const snapshot=session.getSnapshot(),adapter=session.adapter;
      const current=adapter.merge(editing.draft,draft,snapshot.content).value;
      const draftBaseline=projectStructuredFieldInput(adapter,editing.draftBaseline,editing.draft,draft,editing.id);
      const baseline=adapter.merge(editing.draftBaseline,draftBaseline,snapshot.baselineProjection).value;
      if(!perform(()=>session.edit(current,baseline)))return;
      setEditing({...editing,text,draft,draftBaseline});void session.preserve();
    }catch(reason){session.record(`MODEL_FIELD_EDIT_FAILED：${reason instanceof Error?reason.message:String(reason)}`);}
  };
  const save=async()=>{
    if(!editing||disabled)return;
    if(submissionBlocked){session.record(submissionBlocked);return;}
    setSaving(true);
    try{
      await session.save();
      const shown=session.getSnapshot();
      // Saved candidates remain pending. Close the form to expose any inline
      // conflict after S′ has been durably preserved, even if S rejects a CAS.
      if(!shown.preservationError)setEditing(undefined);
    }catch(reason){session.record(`MODEL_FIELD_EDIT_FAILED：${reason instanceof Error?reason.message:String(reason)}`);}
    finally{setSaving(false);}
  };
  const commitOnBlur=(target?:EventTarget|null)=>{
    // Review controls (including their portaled menu) retain the live input
    // until the user's decision. Ordinary focus departure still auto-saves.
    if(isReviewTarget(target))return;
    if(target instanceof Element&&surface.current?.contains(target)&&target.closest(".structured-field-inline-editor"))return;
    return save();
  };
  const active=changes.some(change=>change.id===activeId)?activeId:changes[0]?.id;
  const move=(direction:"previous"|"next")=>{
    if(!changes.length)return;const index=Math.max(0,changes.findIndex(change=>change.id===active));
    const target=changes[(index+(direction==="next"?1:-1)+changes.length)%changes.length];if(!target)return;setActiveId(target.id);
    const field=[...fields,...beforeFields].find(unit=>contains(unit.path,target.path));
    const card=field&&[...surface.current?.querySelectorAll<HTMLElement>("[data-field-id]")??[]].find(card=>card.dataset.fieldId===field.field.id);
    card?.scrollIntoView({block:"center"});card?.focus({preventScroll:true});
  };
  const handled=new Set<string>();
  for(const unit of [...fields,...beforeFields])for(const change of forField(unit.path))handled.add(change.id);
  for(const item of placed)handled.add(item.decisionId);
  return <EditorDiffHighlight appearance={configuration?.highlight}><section ref={surface} className="structured-resource-review hbs-structured-editor" data-structured-resource={resourceId} aria-label={title}
    onPointerDownCapture={event=>{
      if(editing&&event.button===0&&event.target instanceof Element&&event.target.closest("button")&&isReviewTarget(event.target))event.preventDefault();
    }}
    onBlur={event=>{if(editing&&isReviewTarget(event.target))commitOnBlur(event.relatedTarget);}}
    onKeyDown={event=>{
      if(panelOwnsHistory||editing||!(event.metaKey||event.ctrlKey)||event.altKey||event.target instanceof Element&&event.target.closest('input,textarea,[contenteditable="true"]'))return;
      const key=event.key.toLowerCase();if(key!=="z"&&key!=="y")return;const editor=nativeHistory();if(!editor)return;
      event.preventDefault();event.stopPropagation();if(key==="y"||event.shiftKey)redo(editor);else undo(editor);
    }}>
    <div className="structured-resource-cards">
      {changes.length>0&&<StructuredReviewControls count={changes.length} active={Math.max(0,changes.findIndex(change=>change.id===active)+1)} busy={disabled}
        onMove={move} onResolve={(action,scope)=>decide((scope==="current"?changes.filter(change=>change.id===active):changes).map(change=>change.id),action)}
        onKeepUnreviewed={()=>{if(submissionBlocked){session.record(submissionBlocked);return;}setEditing(undefined);if(onKeepUnreviewed)onKeepUnreviewed();else decide(changes.map(change=>change.id),"keep");}} metadataActive={!!active&&!fields.some(unit=>changes.some(change=>change.id===active&&contains(unit.path,change.path)))}/>}
      {fields.map(unit=>{
        const field=unit.field,before=beforeFields.find(value=>value.field.id===field.id)?.field,current=placed.filter(item=>item.fragment.path&&same(item.fragment.path,unit.path));
        const selected=forField(unit.path),conflicts=state.conflicts.filter(conflict=>contains(unit.path,conflict.path));
        const nested=nestedOld.filter(item=>same(item.field.path,unit.path));
        const leaves=structuredTextLeaves(field.value,unit.path),isEditing=editing?.id===field.id;
        // Display the adapter's own authorized incoming branch. In particular,
        // a baseline conflict must not render B as though it were incoming S.
        const incoming=conflicts.length?conflicts.reduce((value,conflict)=>session.adapter.resolveConflict
          ?session.adapter.resolveConflict({content:value,baseline:state.baselineProjection,ancestor:state.reviewAncestor,
            baselineAncestor:state.baselineAncestor,confirmed:state.observed.baseline,saved:state.observed.saved},conflict,"incoming").content
          :session.adapter.replace(value,conflict.path,conflict.incoming),state.content):undefined;
        const incomingField=incoming===undefined?undefined:structuredResourceDiffFields(incoming).find(value=>value.id===field.id);
        const fieldEditor=(content?:ReactNode,inlineDiff=false)=>isEditing?<StructuredFieldTextEditor label={`编辑${field.title}正文`} value={editing.text} disabled={disabled} inlineDiff={inlineDiff}
          {...(editing.initialCaret===undefined?{}:{initialCaret:editing.initialCaret})} onChange={changeText} onCommit={commitOnBlur}
          onHistory={direction=>{if(onHistoryCommand)onHistoryCommand(direction);else nativeActions[direction]();}}>{content}</StructuredFieldTextEditor>:null;
        return <Fragment key={field.id}>
          {fieldOld.filter(item=>{const anchor=structuredOldSuccessor(item,placed);return anchor&&same(anchor,unit.path);}).map(old)}
          <div data-review-location={current.length?"current":undefined} data-hbs-review-decision={current.map(item=>item.decisionId).join(" ")||undefined}>
          <div className={current.length?"editor-diff-added-block":undefined}>
          <StructuredFieldCard id={field.id} title={field.title} appearance={resourceKind==="structuredReading"?"reading":"plain"} {...(field.color?{color:field.color}:{})} pending={!!selected.length||!!current.length}
            onFocus={()=>{if(selected[0])setActiveId(selected[0].id);}} indicators={renderIndicators?.(field)}
            actions={!field.id.startsWith("$")&&<ControlHint content="编辑字段"><button type="button" className="reading-icon-action" aria-label={`编辑${field.title}`} disabled={disabled} onClick={()=>startEditing(field)}><Pencil size={14} aria-hidden="true"/></button></ControlHint>}>
            {isEditing&&!selected.length?fieldEditor()
              :nested.length&&!isEditing?<>{leaves.map(leaf=>{
                const locations=placed.filter(item=>item.fragment.path&&!same(item.fragment.path,unit.path)&&contains(item.fragment.path,leaf.path));
                return <Fragment key={JSON.stringify(leaf.path)}>{nested.filter(item=>{const anchor=structuredOldSuccessor(item,placed);return anchor&&contains(anchor,leaf.path)&&same(leaves.find(candidate=>contains(anchor,candidate.path))?.path??[],leaf.path);}).map(old)}
                  <div data-review-location={locations.length?"current":undefined} data-hbs-review-decision={locations.map(item=>item.decisionId).join(" ")||undefined}><div className={locations.length?"editor-diff-added-block":undefined}>
                    {selected.some(change=>contains(leaf.path,change.path)||contains(change.path,leaf.path))
                      ?<StructuredTextDiff before={String(session.adapter.read(state.baselineProjection,leaf.path)??"")} after={leaf.text}
                        {...(incoming!==undefined&&conflicts.some(change=>contains(leaf.path,change.path)||contains(change.path,leaf.path))?{incoming:String(session.adapter.read(incoming,leaf.path)??"")}:{})}/>:<p>{leaf.text}</p>}
                  </div>{locations.map(item=>locationActions(item,"after"))}</div>
                </Fragment>;
              })}{nested.filter(item=>!structuredOldSuccessor(item,placed)).map(old)}</>
              :selected.length?<StructuredTextDiff before={before?display(before,resourceKind):""} after={display(field,resourceKind)} {...(incomingField?{incoming:display(incomingField,resourceKind)}:{})}
                onEditAfter={event=>startEditing(field,event)} {...(isEditing?{renderAfter:(content:ReactNode)=>fieldEditor(content,!!(before&&display(before,resourceKind)!==display(field,resourceKind))||!!incomingField)}:{})}/>
              :<p onDoubleClick={event=>startEditing(field,event)}>{display(field,resourceKind)||"已读取材料中未找到"}</p>}
            {renderFooter?.(field)}{controls(selected)}
          </StructuredFieldCard></div>{current.map(item=>locationActions(item,"after"))}</div>
        </Fragment>;
      })}{fieldOld.filter(item=>!structuredOldSuccessor(item,placed)).map(old)}
    </div>
    {decisions.filter(({change})=>!handled.has(change.id)).map(({change,conflict,projection})=><div className="hbs-structural-change" key={change.id}>
      <p>{projection.fallbackReason??"概况、模板或来源范围信息已修改。"}</p>
      {conflict&&<p className="hbs-structured-incoming" aria-label="incoming">此信息存在外部更新，请核对当前资源后选择保留方案。</p>}{controls([change])}
    </div>)}
    {state.preservationError&&<p role="alert">{state.preservationError}</p>}
    {state.message.startsWith("MODEL_")&&<p role="status">{state.message}</p>}
    <div hidden ref={historyHost}><HbsLeafEditor session={session} path={["kind"]} label="资源格式" presentation="prose" readOnly readOnlyHistory historyActions={historyActions} stateRetention={stateRetention}/></div>
  </section></EditorDiffHighlight>;
}
