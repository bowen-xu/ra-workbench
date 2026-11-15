// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { EditorReviewLocationActions } from "./EditorReviewLocationActions";

const cleanups:(()=>Promise<void>)[]=[];
afterEach(async()=>{for(const cleanup of cleanups.splice(0))await cleanup();vi.unstubAllGlobals();});
async function setup(side:"before"|"after",disabled=false){
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host),jump=vi.fn(),resolve=vi.fn();
  cleanups.push(async()=>{await act(async()=>root.unmount());host.remove();});
  await act(async()=>root.render(<EditorReviewLocationActions side={side} disabled={disabled} onJump={jump} onResolve={resolve}/>));
  const buttons=()=>[...host.querySelectorAll("button")];
  return {host,root,jump,resolve,buttons};
}
it.each(["before","after"] as const)("keeps navigation independent from the %s location's review action",async side=>{
  const f=await setup(side),buttons=f.buttons();
  expect(buttons.map(button=>button.textContent)).toEqual(side === "before" ? ["new","Undo"] : ["old","Keep"]);
  await act(async()=>buttons[0]!.click());
  expect(f.jump).toHaveBeenCalledOnce();expect(f.resolve).not.toHaveBeenCalled();
  await act(async()=>buttons[1]!.click());
  expect(f.resolve).toHaveBeenCalledExactlyOnceWith(side === "before" ? "undo" : "keep");
  expect(f.jump).toHaveBeenCalledOnce();
});
it("uses configured review labels without changing navigation intent",async()=>{
  const f=await setup("after");
  await act(async()=>f.root.render(<EditorReviewLocationActions side="after" onJump={f.jump} onResolve={f.resolve}
    configuration={{labels:{keep:"采纳"}}}/>));
  expect(f.buttons().map(button=>button.textContent)).toEqual(["old","采纳"]);
  await act(async()=>f.buttons()[1]!.click());
  expect(f.resolve).toHaveBeenCalledExactlyOnceWith("keep");
});
it("disables controls without emitting navigation or review",async()=>{
  const f=await setup("before",true);
  expect(f.buttons().every(button=>button.disabled)).toBe(true);
  await act(async()=>f.buttons().forEach(button=>button.click()));
  expect(f.jump).not.toHaveBeenCalled();expect(f.resolve).not.toHaveBeenCalled();
});
