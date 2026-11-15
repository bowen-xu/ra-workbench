import { useEffect, useRef } from "react";
import { createEditorDiffChunkActions, updateEditorDiffChunkActions } from "./editorDiffChunkActions";
import "./EditorDiffControls.css";
import { EditorReviewActionGroup, type EditorReviewAction, type EditorReviewConfiguration } from "./EditorReviewActions";
/** React host for the same controls used by the text and note editor widgets. */
export function EditorDiffChunkActions(props: {
  readonly number?: number; readonly disabled?: boolean;
  readonly configuration?: EditorReviewConfiguration | undefined;
} & ({ readonly conflict: true; readonly onResolve: (action: EditorReviewAction) => void }
  | { readonly conflict?: false; readonly onResolve: (action: "keep" | "undo") => void })) {
  const { number = 1, disabled = false, configuration } = props;
  const host=useRef<HTMLDivElement>(null),row=useRef<HTMLElement|undefined>(undefined);
  const latest=useRef(props);latest.current=props;
  useEffect(()=>{
    if(!host.current || props.conflict)return;
    const element=createEditorDiffChunkActions({document:host.current.ownerDocument,number,disabled,
      ...(configuration?.labels ? { labels: configuration.labels } : {}),
      onResolve:action=>{ if(!latest.current.conflict)latest.current.onResolve(action); }});
    row.current=element;host.current.append(element);
    return()=>{element.remove();row.current=undefined;};
  },[props.conflict, configuration?.labels]);
  useEffect(()=>{if(row.current)updateEditorDiffChunkActions(row.current,{number,disabled});},[number,disabled]);
  if (props.conflict) return <EditorReviewActionGroup onResolve={props.onResolve} disabled={disabled} configuration={configuration} />;
  return <div ref={host}/>;
}
