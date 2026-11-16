/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { emptyNalState } from "@research-assistant/core/nal";
import { selectAppRuntime } from "../../../../../../apps/web/src/core/runtime/appRuntime";
import { NalSidebar } from "../../../../../../../plugins/ra-literature-library/src/features/nal/NalSidebar";
import type { NalController } from "../../../../../../../plugins/ra-literature-library/src/features/nal/useNal";

let host: HTMLDivElement, root: Root;
beforeEach(()=>{
  (globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
  selectAppRuntime({mode:"browser",storage:window.localStorage,gateways:{},dispose(){}});
  host=document.createElement("div");document.body.append(host);root=createRoot(host);
});
afterEach(async()=>{await act(async()=>root.unmount());host.remove();});

it("selects definitions without changing the list, executing a query, or clearing an existing query", async()=>{
  const state={...emptyNalState(),concepts:[{id:"a",label:"领域",explanation:"领域定义",revision:1},{id:"b",label:"方法",explanation:"方法定义",revision:1}]};
  const controller:NalController={state,ready:true,busy:false,outputVariables:undefined,query:[{subject:{kind:"variable",name:"文章"},predicate:{kind:"concept",id:"a"}}],
    setQuery:vi.fn(),refresh:vi.fn(async()=>true),mutate:vi.fn(async()=>true),vocabulary:[],matches:[],allowed:new Set(),collections:[],selectedCollectionId:undefined,selectCollection:vi.fn(),saveCollection:vi.fn(async()=>true),removeCollection:vi.fn(async()=>true),editCollections:vi.fn(async()=>true),cacheError:undefined};
  const render=async(value:NalController)=>act(async()=>root.render(<NalSidebar controller={value} onAnnotate={()=>{}}/>));
  await render(controller);
  expect(host.querySelector(".nal-query-summary")).toBeNull();
  expect(host.querySelector('nav[aria-label="概念列表"]')).toBeNull();
  await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="管理概念与关系"]')!.click());
  expect(host.querySelector('[aria-label="查询集合"]')).toBeNull();
  const buttons=()=>[...host.querySelectorAll<HTMLButtonElement>('nav[aria-label="概念列表"] button')];
  expect(buttons().map(b=>b.textContent)).toEqual(["领域","方法"]);
  await act(async()=>buttons()[1]!.click());
  expect(host.querySelector(".nal-concept-detail strong")?.textContent).toBe("方法");
  expect(buttons().map(b=>b.textContent)).toEqual(["领域","方法"]);
  expect(host.querySelector(".nal-sidebar > textarea")).toBeNull();
  expect(host.querySelector('[aria-label="设置文献查询"]')).not.toBeNull();
  expect(controller.setQuery).not.toHaveBeenCalled();
  await render({...controller,state:{...state,concepts:[state.concepts[0]!]}});
  expect(controller.setQuery).not.toHaveBeenCalled();
  await render({...controller,cacheError:"读取失败"});
  expect(host.querySelector('[role="status"]')?.textContent).toContain("读取失败");
});
