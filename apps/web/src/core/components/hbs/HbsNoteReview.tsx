import { Fragment, useEffect, useRef, useState, useSyncExternalStore, type MutableRefObject } from "react";
import { createPortal } from "react-dom";
import type { DocumentChange, DocumentReviewProjector, HbsSession } from "@research-assistant/core/hbs";
import type { NoteDocumentV1 } from "@research-assistant/core/notes";
import type { NoteEditorInstance } from "../../features/notes/editor/noteEditorSchema";
import type { NoteEditorInstallPort } from "../../features/notes/editor/noteEditorInstallPort";
import { EditorDiffChunkActions } from "../editor-diff/EditorDiffChunkButtons";
import { EditorReviewLocationActions } from "../editor-diff/EditorReviewLocationActions";
import { EditorDiffControls } from "../editor-diff/EditorDiffControls";
import type { EditorReviewAction, EditorReviewConfiguration } from "../editor-diff/EditorReviewActions";
import { appendSessionHistory } from "./noteSessionHistory";
import { createNoteUnitDecorations } from "./noteUnitDecorations";
import type { NoteReviewLocation } from "./noteReviewProjection";
import type { NoteTableComparison } from "./noteTableDecorations";
import {useHbsReviewCommands} from "./useHbsReviewCommands";
import "../../features/notes/editor/NoteEditorDiff.css";
import "./HbsDocumentEditor.css";
interface ActionHost { element:HTMLElement;changes:readonly DocumentChange[];conflict:boolean;number:number;location?:NoteReviewLocation }
export function HbsNoteReview({editor,port,session,configuration,tableComparison,reviewProjector,disabled=false,onRetrySave,retryRequired=false,controlsTarget}:{readonly editor:NoteEditorInstance;readonly port:MutableRefObject<NoteEditorInstallPort|undefined>;
  readonly session:HbsSession<NoteDocumentV1>;readonly configuration?:EditorReviewConfiguration;readonly tableComparison?:NoteTableComparison;readonly reviewProjector?:DocumentReviewProjector;
  readonly disabled?:boolean;readonly onRetrySave?:(()=>Promise<void>)|undefined;readonly retryRequired?:boolean;readonly controlsTarget?:HTMLElement|null|undefined}): React.JSX.Element {
  const state=useSyncExternalStore(session.subscribe,session.getSnapshot),live=useRef(state);live.current=state;
  const runReview=useHbsReviewCommands(session,state,{onRetrySave,retryRequired});
  const [hosts,setHosts]=useState<readonly ActionHost[]>([]);
  const [active,setActive]=useState(0);
  const decoration=useRef<ReturnType<typeof createNoteUnitDecorations>|undefined>(undefined);
  const refreshHosts=useRef<()=>void>(()=>{});
  useEffect(()=>{
    let disposed=false;let actionHosts:ActionHost[]=[];
    const refresh=()=>queueMicrotask(()=>{
      if(disposed)return;
      const pending=new Set([...live.current.changes,...live.current.conflicts].map(change=>change.id));
      actionHosts=actionHosts.filter(host=>host.element.isConnected&&host.changes.every(change=>pending.has(change.id)));
      setHosts(previous=>previous.length===actionHosts.length&&previous.every((host,index)=>host===actionHosts[index])?previous:[...actionHosts]);
      if(!actionHosts.length)setActive(0);
    });
    refreshHosts.current=refresh;
    const plugin=createNoteUnitDecorations(editor,live.current,(owner,changes,conflict,number,location)=>{
      const element=owner.createElement("div");element.className=`note-diff-chunk-actions${conflict?" editor-diff-chunk-actions writing-diff-chunk-actions":""}`;
      element.contentEditable="false";actionHosts.push({element,changes,conflict,number,...(location?{location}:{})});
      refresh();return element;
    },(change,current)=>{
      const state=session.getSnapshot();
      return session.adapter.resolveConflict?.({content:current,baseline:state.baselineProjection,ancestor:state.ancestor,
        baselineAncestor:state.baselineAncestor,confirmed:state.observed.baseline,saved:state.observed.saved},change,"incoming").content
        ??session.adapter.replace(current,change.path,change.incoming);
    },tableComparison,reviewProjector);decoration.current=plugin;
    const attach=()=>{if(disposed||!editor.domElement)return;
      if(!plugin.key.getState(editor.prosemirrorState))editor._tiptapEditor.registerPlugin(plugin.plugin);
      editor.prosemirrorView.dispatch(editor.prosemirrorView.state.tr.setMeta(plugin.key,live.current).setMeta("addToHistory",false));};
    editor._tiptapEditor.on("mount",attach);const frame=requestAnimationFrame(attach);attach();
    return()=>{disposed=true;refreshHosts.current=()=>{};cancelAnimationFrame(frame);editor._tiptapEditor.off("mount",attach);
      if(!editor._tiptapEditor.isDestroyed)editor._tiptapEditor.unregisterPlugin(plugin.key);decoration.current=undefined;};
  },[editor,session,tableComparison,reviewProjector]);
  useEffect(()=>{const plugin=decoration.current;if(plugin&&editor.domElement)
    editor.prosemirrorView.dispatch(editor.prosemirrorView.state.tr.setMeta(plugin.key,state).setMeta("addToHistory",false));
    refreshHosts.current();},[editor,state]);
  const resolve=(ids:readonly string[],action:EditorReviewAction)=>{
    if(disabled)return;
    runReview(action,()=>{
    const installer=port.current;
    if(!installer){session.record("MODEL_EDITOR_NOT_READY：编辑器安装端口尚未就绪，输入保留。");return;}
    try{const entry=session.perform(()=>session.resolveMany(ids,action));
      installer.prepareStructure(installer.read(),session.getSnapshot().content,undefined,{onTransaction:transaction=>appendSessionHistory(transaction,entry)})();editor.focus();
    }catch(reason){session.record(`MODEL_EDITOR_INSTALL_FAILED：${reason instanceof Error?reason.message:String(reason)}`);}
    });
  };
  const pending=new Set([...state.changes,...state.conflicts].map(change=>change.id));
  const visibleHosts=hosts.filter(host=>host.element.isConnected&&host.changes.every(change=>pending.has(change.id)));
  const units=[...new Map(visibleHosts.map(host=>[host.changes.map(change=>change.id).join("|"),host])).values()];
  const current=Math.min(active,Math.max(0,units.length-1));
  const controls=units.length?<div className="note-editor-diff-controls"><EditorDiffControls count={units.length} active={current+1} disabled={disabled}
    showBulk={configuration?.showBulk!==false} onMove={direction=>{const next=current+(direction==="next"?1:-1);const unit=units[next];if(!unit)return;
      setActive(next);unit.element.scrollIntoView({block:"center"});}}
    onResolve={(action,scope)=>resolve([...new Set((scope==="all"?units:[units[current]!]).flatMap(host=>host.changes.map(change=>change.id)))],action)}/></div>:null;
  // Standalone editors use the same controls above their content. Retained,
  // inactive hosts explicitly pass null; active page hosts provide a top slot.
  return <>{controlsTarget?createPortal(controls,controlsTarget):controlsTarget===undefined?controls:null}{visibleHosts.map(host=><Fragment key={host.changes.map(change=>change.id).join("|")+String(host.conflict)+(host.location ? `${host.location.key}:${host.location.side}` : "")}>{createPortal(host.location
    ?<EditorReviewLocationActions side={host.location.side} onJump={host.location.jump} disabled={disabled} configuration={configuration} onResolve={action=>resolve(host.changes.map(change=>change.id),action)}/>
    :host.conflict
    ?<EditorDiffChunkActions conflict number={host.number} disabled={disabled} configuration={configuration} onResolve={action=>resolve(host.changes.map(change=>change.id),action)}/>
    :<EditorDiffChunkActions number={host.number} disabled={disabled} configuration={configuration} onResolve={action=>resolve(host.changes.map(change=>change.id),action)}/>,host.element)}</Fragment>)}</>;
}
