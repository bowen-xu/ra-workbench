// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { EditorView } from "@codemirror/view";
import type { EditorState } from "@codemirror/state";
import { undo, redo, undoDepth } from "@codemirror/commands";
import { expect,it,vi } from "vitest";
import { TextEditor } from "./TextEditor";

it("retains native history across host handoff while binding all input callbacks to the current host",async()=>{
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  let retained:EditorState|undefined;
  const stateRetention={load:()=>retained,save:(state:EditorState)=>{retained=state;}};
  const first=vi.fn(),second=vi.fn();
  const native=()=>EditorView.findFromDOM(host.querySelector<HTMLElement>(".cm-editor")!)!;
  try{
    await act(async()=>root.render(<TextEditor documentId="retained" path="字段" value="old" baseline="old" onChange={first} stateRetention={stateRetention}/>));
    await act(async()=>native().dispatch({changes:{from:0,to:3,insert:"typed"},userEvent:"input.type"}));
    expect(first).toHaveBeenCalledWith("typed","typed");
    const old=native();expect(undoDepth(old.state)).toBe(1);
    await act(async()=>root.render(null));expect(retained).toBe(old.state);
    first.mockClear();
    await act(async()=>root.render(<TextEditor documentId="retained" path="字段" value="typed" baseline="typed" onChange={second} stateRetention={stateRetention}/>));
    expect(native()).not.toBe(old);expect(undoDepth(native().state)).toBe(1);
    await act(async()=>{expect(undo(native())).toBe(true);});
    expect(second).toHaveBeenCalledWith("old","old");expect(first).not.toHaveBeenCalled();
    await act(async()=>{expect(redo(native())).toBe(true);});
    expect(second).toHaveBeenLastCalledWith("typed","typed");expect(first).not.toHaveBeenCalled();
  }finally{await act(async()=>root.unmount());host.remove();vi.unstubAllGlobals();}
});
