// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { undo,redo,undoDepth } from "@codemirror/commands";
import { EditorView } from "@codemirror/view";
import { afterEach,expect,it,vi } from "vitest";
import { TextEditor } from "./TextEditor";
import type { TextEditorHistoryPort } from "./textEditorHistory";

const cleanups:(()=>Promise<void>)[]=[];
afterEach(async()=>{for(const cleanup of cleanups.splice(0))await cleanup();vi.unstubAllGlobals();});
it("keeps read-only DOM and rejects edits while native structural history can undo and redo",async()=>{
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  cleanups.push(async()=>{await act(async()=>root.unmount());host.remove();});
  let model=0,run:((action:()=>void)=>void)|undefined;
  const value="actual_resource_kind",onChange=vi.fn();
  const point=(count:number)=>({value,baseline:value,restore:()=>{model=count;}});
  const history:TextEditorHistoryPort={perform:action=>{const before=point(model);action();return {before,after:point(model)};}};
  const actions={bind:(perform:(action:()=>void)=>void)=>{run=perform;return()=>{run=undefined;};}};
  const render=async(content=value)=>{await act(async()=>root.render(<TextEditor documentId="readonly-structure" path="资源格式"
    value={content} baseline={content} readOnly readOnlyHistory presentation="prose" onChange={onChange} history={history} historyActions={actions}/>));};
  await render();
  const editor=EditorView.findFromDOM(host.querySelector<HTMLElement>(".cm-editor")!);
  if(!editor||!run)throw new Error("history owner not ready");
  const perform=run;
  expect(editor.contentDOM.contentEditable).toBe("false");expect(editor.contentDOM.getAttribute("aria-readonly")).toBe("true");
  for(const event of [undefined,"input.type","input.paste","delete.backward","input.replace"]){
    await act(async()=>editor.dispatch({changes:{from:0,to:editor.state.doc.length,insert:"forbidden"},...(event?{userEvent:event}:{})}));
    expect(editor.state.doc.toString()).toBe(value);
  }
  expect(onChange).not.toHaveBeenCalled();
  await act(async()=>perform(()=>{model=1;}));
  expect(model).toBe(1);expect(undoDepth(editor.state)).toBe(1);
  await act(async()=>{expect(undo(editor)).toBe(true);});expect(model).toBe(0);
  await act(async()=>{expect(redo(editor)).toBe(true);});expect(model).toBe(1);
  const depth=undoDepth(editor.state);
  await render("externally_updated_kind");
  expect(EditorView.findFromDOM(host.querySelector<HTMLElement>(".cm-editor")!)).toBe(editor);
  expect(editor.state.doc.toString()).toBe("externally_updated_kind");expect(undoDepth(editor.state)).toBe(depth);
  expect(editor.contentDOM.contentEditable).toBe("false");
});
