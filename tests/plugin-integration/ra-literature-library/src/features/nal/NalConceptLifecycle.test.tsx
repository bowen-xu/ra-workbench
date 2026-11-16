/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { emptyNalState } from "@research-assistant/core/nal";
import { selectAppRuntime } from "../../../../../../apps/web/src/core/runtime/appRuntime";
import { NalSidebar } from "../../../../../../../plugins/ra-literature-library/src/features/nal/NalSidebar";
import { NalConceptEditor } from "../../../../../../../plugins/ra-literature-library/src/features/nal/NalConceptEditor";
import type { NalController } from "../../../../../../../plugins/ra-literature-library/src/features/nal/useNal";

let host: HTMLDivElement, root: Root;
function controller(): NalController {
  return {
    state:{...emptyNalState(),concepts:[{id:"a",label:"领域",explanation:"",revision:1}]},
    ready:true,busy:false,query:undefined,outputVariables:undefined,setQuery:vi.fn(),refresh:vi.fn(async()=>true),
    mutate:vi.fn(async()=>true),vocabulary:[],matches:[],allowed:new Set(),collections:[],
    selectedCollectionId:undefined,selectCollection:vi.fn(),saveCollection:vi.fn(async()=>true),
    removeCollection:vi.fn(async()=>true),editCollections:vi.fn(async()=>true),cacheError:undefined,
  };
}
beforeEach(()=>{
  (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
  localStorage.clear();
  selectAppRuntime({mode:"browser",storage:window.localStorage,gateways:{},dispose(){}});
  host=document.createElement("div");document.body.append(host);root=createRoot(host);
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();vi.restoreAllMocks();});

it("confirms deletion, leaves cancellation unchanged, and removes the selected detail after refresh",async()=>{
  const value=controller(),confirm=vi.fn(()=>false);
  Object.defineProperty(window,"confirm",{configurable:true,value:confirm});
  const render=async(c:NalController)=>act(async()=>root.render(<NalSidebar controller={c} onAnnotate={()=>{}}/>));
  await render(value);
  expect(host.querySelector('nav[aria-label="概念列表"]')).toBeNull();
  await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="管理概念与关系"]')!.click());
  await act(async()=>host.querySelector<HTMLButtonElement>('nav[aria-label="概念列表"] button')!.click());
  const remove=host.querySelector<HTMLButtonElement>('[aria-label="删除概念：领域"]')!;
  await act(async()=>remove.click());
  expect(value.mutate).not.toHaveBeenCalled();
  confirm.mockReturnValue(true);
  await act(async()=>remove.click());
  expect(value.mutate).toHaveBeenCalledWith({kind:"delete_concept",id:"a"},value.state.revision);
  await render({...value,state:{...value.state,concepts:[]}});
  expect(host.querySelector(".nal-concept-detail")).toBeNull();
});

it("marks explanations optional while retaining the required name",async()=>{
  await act(async()=>root.render(<NalConceptEditor controller={controller()} onCreated={()=>{}} onClose={()=>{}}/>));
  const dialog=document.querySelector('dialog')!;
  expect(dialog.querySelector<HTMLTextAreaElement>('[aria-label="解释"]')!.required).toBe(false);
  expect(dialog.querySelector<HTMLInputElement>('[aria-label="名称"]')!.required).toBe(true);
  expect(dialog.textContent).toContain("解释（选填）");
});
