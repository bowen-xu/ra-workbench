// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { DEFAULT_PORTRAIT_CATALOG } from "@research-assistant/core/ai";
import type { StructuredResource } from "@research-assistant/core/structured-resources";
import { createMockStructuredResources } from "../../../../../../../../integration/web/src/app/runtime/mock/mockStructuredResources";
import { StructuredPortraitEditor } from "../../../../../../../../plugins/ra-literature-library/src/features/ai/resources/StructuredPortraitEditor";

(globalThis as {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
vi.mock("../../../../../../../apps/web/src/core/services/notifications",()=>({notify:{error:vi.fn(),success:vi.fn()}}));
let dispose:(()=>Promise<void>)|undefined;
afterEach(async()=>{await dispose?.();dispose=undefined;document.body.innerHTML="";});
async function setup(){
  const values=new Map<string,string>();
  const storage:Storage={get length(){return values.size;},key:index=>[...values.keys()][index]??null,
    getItem:key=>values.get(key)??null,setItem:(key,value)=>{values.set(key,value);},removeItem:key=>{values.delete(key);},clear:()=>values.clear()};
  const gateway=createMockStructuredResources(storage);
  const target=(await gateway.list("paper")).find(value=>value.kind==="portrait")!;
  const content={kind:"paper_portrait_v1",schemaVersion:1,coverage:"来源摘录",template:[],
    claims:["genre","mode","paradigm"].map(dimension=>({id:`claim_${dimension}`,dimension,
      label:dimension==="genre"?"原创研究":"不确定",description:"描述",certainty:"uncertain",evidence:[]}))};
  const initial=(await gateway.write({resourceId:target.id,expectedRevision:0,requestId:"initial",
    editorKind:"human",content:JSON.stringify(content)})).resource;
  let install:(value:StructuredResource)=>void=()=>{};
  function Host(){const [resource,setResource]=useState(initial);install=setResource;
    return <StructuredPortraitEditor resource={resource} catalog={DEFAULT_PORTRAIT_CATALOG}
      gateway={gateway} onChanged={setResource} onSaveTags={()=>true}/>;}
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  dispose=()=>act(async()=>root.unmount());
  await act(async()=>root.render(<Host/>));
  const button=(label:string)=>host.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`)!;
  const click=async(label:string)=>{await act(async()=>button(label).click());};
  return {host,gateway,storage,target,initial,button,click,install};
}

it("saves each classification change immediately and undoes/redoes the durable content with buttons and shortcuts",async()=>{
  const f=await setup();
  expect(f.host.textContent).not.toContain("保存画像修改");expect(f.host.textContent).not.toContain("取消编辑");
  await f.click("移除原创研究标签");
  expect((await f.gateway.get(f.target.id)).revision).toBe(f.initial.revision+1);
  expect(f.button("移除原创研究标签")).toBeNull();
  await f.click("撤销画像分类修改");expect(f.button("移除原创研究标签")).not.toBeNull();
  await f.click("重做画像分类修改");expect(f.button("移除原创研究标签")).toBeNull();
  const region=f.host.querySelector('[aria-label="编辑当前画像分类"]')!;
  const undo=new KeyboardEvent("keydown",{key:"z",metaKey:true,bubbles:true,cancelable:true});
  await act(async()=>region.dispatchEvent(undo));
  expect(undo.defaultPrevented).toBe(true);expect(f.button("移除原创研究标签")).not.toBeNull();
  await act(async()=>region.dispatchEvent(new KeyboardEvent("keydown",{key:"z",metaKey:true,shiftKey:true,bubbles:true,cancelable:true})));
  expect(f.button("移除原创研究标签")).toBeNull();
  const reopened=createMockStructuredResources(f.storage);
  expect(await reopened.get(f.target.id)).toEqual(await f.gateway.get(f.target.id));
});

it("does not record failed saves in history and drops stale undo after an external version arrives",async()=>{
  const f=await setup();
  vi.spyOn(f.gateway,"write").mockRejectedValueOnce(new Error("保存失败"));
  await f.click("移除原创研究标签");
  expect(f.button("移除原创研究标签")).not.toBeNull();
  expect(f.button("撤销画像分类修改").disabled).toBe(true);
  await f.click("移除原创研究标签");
  const current=await f.gateway.get(f.target.id);
  const outside=(await f.gateway.write({resourceId:current.id,expectedRevision:current.revision,
    requestId:"external",editorKind:"agent",content:JSON.stringify({...f.initial.content as object,coverage:"外部更新"})})).resource;
  await act(async()=>f.install(outside));
  expect(f.button("撤销画像分类修改").disabled).toBe(true);
});
