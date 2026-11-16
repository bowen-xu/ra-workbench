// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { DEFAULT_PORTRAIT_CATALOG } from "@research-assistant/core/ai";
import type { StructuredResource } from "@research-assistant/core/structured-resources";
import { createMockStructuredResources } from "../../../../../../../integration/web/src/app/runtime/mock/mockStructuredResources";
import { PortraitEditHistory, PortraitHistoryControls, PortraitHistoryProvider, portraitHistoryKeyDown, usePortraitEditHistory } from "../../../../../../../plugins/ra-literature-library/src/features/ai/PortraitEditHistory";
import type { AppHbsDocumentGateway } from "../../../../../../apps/web/src/core/services/hbsDocumentGateway";
import { StructuredPortraitEditor } from "../../../../../../../plugins/ra-literature-library/src/features/ai/resources/StructuredPortraitEditor";
import { ManagedReadingTemplate } from "../../../../../../../plugins/ra-literature-library/src/features/ai/resources/ManagedReadingTemplate";
import { StructuredResourceReview } from "../../../../../../../plugins/ra-literature-library/src/features/ai/resources/StructuredResourceReview";

(globalThis as {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
vi.mock("../../../../../../apps/web/src/core/services/notifications",()=>({notify:{error:vi.fn(),success:vi.fn()}}));
let dispose:(()=>Promise<void>)|undefined;
afterEach(async()=>{await dispose?.();dispose=undefined;document.body.innerHTML="";vi.restoreAllMocks();});
async function setup(){
  const values=new Map<string,string>();
  const storage:Storage={get length(){return values.size;},key:index=>[...values.keys()][index]??null,
    getItem:key=>values.get(key)??null,setItem:(key,value)=>{values.set(key,value);},removeItem:key=>{values.delete(key);},clear:()=>values.clear()};
  const gateway={...createMockStructuredResources(storage),getSources:async()=>[]};
  const raw:AppHbsDocumentGateway={read:(_kind,id)=>gateway.hbs.read(id),commit:(_kind,id,request)=>gateway.hbs.commit(id,request),
    readPreservation:(_kind,id)=>gateway.hbs.readPreservation(id),savePreservation:(_kind,id,value)=>gateway.hbs.savePreservation(id,value)};
  const all=await gateway.list("paper");
  const portrait=all.find(value=>value.kind==="portrait")!;
  const template=all.find(value=>value.kind==="readingTemplate")!;
  const initial=(await gateway.write({resourceId:portrait.id,expectedRevision:0,requestId:"initial",
    editorKind:"human",content:JSON.stringify({kind:"paper_portrait_v1",schemaVersion:1,coverage:"来源摘录",template:[],
      claims:["genre","mode","paradigm"].map(dimension=>({id:`claim_${dimension}`,dimension,
        label:dimension==="genre"?"原创研究":"不确定",description:"描述",certainty:"uncertain",evidence:[]}))})})).resource;
  const staged=await gateway.write({resourceId:template.id,expectedRevision:0,requestId:"template",
    editorKind:"human",content:JSON.stringify({kind:"reading_template_v1",schemaVersion:1,name:"本文模板",
      fields:[{id:"question",title:"问题",question:"问题定义。",claimIds:[]},{id:"method",title:"方法",question:"方法定义。",claimIds:[]}]})});
  const published=staged.resource;
  let install:(value:StructuredResource)=>void=()=>{};
  function Content(){
    const [p,setPortrait]=useState(initial),[t,setTemplate]=useState(published);
    install=value=>value.kind==="portrait"?setPortrait(value):setTemplate(value);
    const history=usePortraitEditHistory();
    return <section onKeyDown={event=>portraitHistoryKeyDown(event,history,false)} aria-label="画像与模板编辑">
      <header><h3>画像与模板</h3><PortraitHistoryControls disabled={false}/></header>
      <StructuredPortraitEditor resource={p} catalog={DEFAULT_PORTRAIT_CATALOG} gateway={gateway} onChanged={install} onSaveTags={()=>true}/>
      <StructuredResourceReview resource={p} gateway={gateway} hbsGateway={raw} onChanged={install} hideWhenClean/>
      <ManagedReadingTemplate resource={t} gateway={gateway} hbsGateway={raw} onChanged={install}/>
    </section>;
  }
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  dispose=()=>act(async()=>root.unmount());
  await act(async()=>root.render(<PortraitHistoryProvider><Content/></PortraitHistoryProvider>));
  const button=(name:string)=>[...host.querySelectorAll<HTMLButtonElement>("button")].find(value=>value.getAttribute("aria-label")===name||value.textContent===name)!;
  const click=async(name:string)=>{
    await act(async()=>button(name).click());
    await vi.waitFor(async()=>{
      await act(async()=>{});
      expect(host.querySelector('[aria-busy="true"]')).toBeNull();
    });
  };
  const input=async(label:string,value:string)=>{await act(async()=>{
    const element=host.querySelector<HTMLInputElement>(`[aria-label="${label}"]`)!;
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(element,value);
    element.dispatchEvent(new Event("input",{bubbles:true}));
  });};
  const settle=async()=>vi.waitFor(async()=>{await act(async()=>{});expect(host.querySelector('[aria-busy="true"]')).toBeNull();});
  const blur=async(label:string)=>{
    await act(async()=>host.querySelector(`[aria-label="${label}"]`)!.dispatchEvent(new FocusEvent("focusout",{bubbles:true})));
    await settle();
  };
  return {gateway,host,button,click,input,initial,published,install,settle,blur};
}
const undo="撤销画像与模板修改",redo="重做画像与模板修改";
const titles=(host:HTMLElement)=>[...host.querySelectorAll(".portrait-template-list strong")].map(value=>value.textContent);

it("invalidates only a stale form owner and retains another owner's ordered native history",async()=>{
  const history=new PortraitEditHistory(),events:string[]=[];
  const edit=(scope:string)=>({scope,undo:()=>{events.push(`undo ${scope}`);return true;},redo:()=>{events.push(`redo ${scope}`);return true;}});
  history.record(edit("resource:form"));history.record(edit("resource:hbs"));history.record(edit("sources"));
  await history.replay("undo");history.clearScope("resource:form");
  await history.replay("undo");await history.replay("redo");await history.replay("redo");
  expect(events).toEqual(["undo sources","undo resource:hbs","redo resource:hbs","redo sources"]);
  expect(history.canUndo).toBe(true);
});

it("keeps the native review owner mounted after the final Keep and exposes its Undo in the existing panel toolbar",async()=>{
  const f=await setup();
  const current=await f.gateway.get(f.initial.id),content=current.content as {claims:readonly {id:string;description:string}[]};
  const incoming=(await f.gateway.write({resourceId:current.id,expectedRevision:current.revision,requestId:"agent-description",editorKind:"agent",
    content:JSON.stringify({...content,claims:content.claims.map((claim,index)=>index===0?{...claim,description:"新的分类依据"}:claim)})})).resource;
  await act(async()=>f.install(incoming));
  await f.click("保留第 1 处变更");
  const section=f.host.querySelector('[aria-label="文献画像"]');expect(section).not.toBeNull();
  expect(section?.closest("[hidden]")).not.toBeNull();expect(f.button(undo).disabled).toBe(false);
  await f.click(undo);
  expect(f.host.querySelector('[aria-label="文献画像"]')?.closest("[hidden]")).toBeNull();
  expect(f.host.querySelector('[aria-label="文献画像"] .structured-card-diff')).not.toBeNull();
  expect((await f.gateway.get(f.initial.id)).content).toEqual(incoming.content);
});

it("orders classification and autosaved template edits in one toolbar",async()=>{
  const f=await setup();
  expect(f.host.querySelectorAll('[role="toolbar"]')).toHaveLength(1);
  expect(f.host.querySelector('header')?.contains(f.button(undo))).toBe(true);
  await f.click("移除原创研究标签");
  await f.click("添加字段");
  expect(titles(f.host)).toEqual(["问题","方法","新字段"]);
  await f.click(undo);expect(titles(f.host)).toEqual(["问题","方法"]);
  expect(f.button("移除原创研究标签")).toBeUndefined();
  await f.click(undo);expect(f.button("移除原创研究标签")).toBeDefined();
  await f.click(redo);await f.click(redo);
  expect(titles(f.host)).toEqual(["问题","方法","新字段"]);
  expect(f.button("保存模板")).toBeUndefined();
  expect((await f.gateway.get(f.published.id)).content).toMatchObject({fields:expect.arrayContaining([expect.objectContaining({title:"新字段"})])});
  await f.click(undo);
  expect(titles(f.host)).toEqual(["问题","方法"]);
  expect(((await f.gateway.get(f.published.id)).content as {fields:unknown[]}).fields).toHaveLength(2);
  await f.click(undo);expect(f.button("移除原创研究标签")).toBeDefined();
  await f.click(redo);await f.click(redo);
  expect(((await f.gateway.get(f.published.id)).content as {fields:unknown[]}).fields).toHaveLength(3);
});

it("covers template names, field edits, deletion and keyboard order while leaving input shortcuts native",async()=>{
  const f=await setup();
  await f.input("模板名称","改名");
  await f.click("编辑问题");
  expect(f.button(undo).disabled).toBe(true);
  await f.input("结构标题","研究问题");await f.click("保存字段");
  await f.click("删除方法字段");
  await f.click(undo);expect(titles(f.host)).toEqual(["研究问题","方法"]);
  await f.click(undo);expect(titles(f.host)).toEqual(["问题","方法"]);
  await f.click(undo);expect(f.host.querySelector<HTMLInputElement>('[aria-label="模板名称"]')?.value).toBe("本文模板");
  const target=f.host.querySelector<HTMLInputElement>('[aria-label="模板名称"]')!;
  const native=new KeyboardEvent("keydown",{key:"z",metaKey:true,bubbles:true,cancelable:true});
  await act(async()=>target.dispatchEvent(native));expect(native.defaultPrevented).toBe(false);
  const handle=f.button("拖动调整问题顺序");
  await act(async()=>handle.dispatchEvent(new KeyboardEvent("keydown",{key:"ArrowDown",bubbles:true,cancelable:true})));
  expect(titles(f.host)).toEqual(["方法","问题"]);
  const region=f.host.querySelector('[aria-label="画像与模板编辑"]')!;
  await act(async()=>region.dispatchEvent(new KeyboardEvent("keydown",{key:"z",ctrlKey:true,bubbles:true,cancelable:true})));
  expect(titles(f.host)).toEqual(["问题","方法"]);
  expect(f.button(redo).disabled).toBe(false);
});

it("keeps a failed saved-template undo retryable and invalidates stale history on external changes",async()=>{
  const f=await setup();
  await f.click("添加字段");await f.blur("模板名称");
  vi.spyOn(f.gateway,"write").mockRejectedValueOnce(new Error("暂时无法保存"));
  await f.click(undo);expect(f.button(undo).disabled).toBe(false);
  await f.click(undo);
  expect(((await f.gateway.get(f.published.id)).content as {fields:unknown[]}).fields).toHaveLength(2);
  await f.click("移除原创研究标签");
  const current=await f.gateway.get(f.initial.id);
  const external=(await f.gateway.write({resourceId:current.id,expectedRevision:current.revision,requestId:"external",
    editorKind:"agent",content:JSON.stringify({...f.initial.content as object,coverage:"外部更改"})})).resource;
  await act(async()=>f.install(external));expect(f.button(undo).disabled).toBe(true);
});

it("autosaves paper-template edits without save/cancel buttons and replays their persisted content",async()=>{
  const f=await setup();
  const write=vi.spyOn(f.gateway,"write");
  expect(f.button("保存模板")).toBeUndefined();expect(f.button("取消未保存修改")).toBeUndefined();
  expect(write).not.toHaveBeenCalled();
  await f.input("模板名称","自动保存名称");expect(write).not.toHaveBeenCalled();
  await f.blur("模板名称");
  expect((await f.gateway.get(f.published.id)).content).toMatchObject({name:"自动保存名称"});
  await f.click("添加字段");
  expect(((await f.gateway.get(f.published.id)).content as {fields:unknown[]}).fields).toHaveLength(3);
  await f.click(undo);expect(titles(f.host)).toEqual(["问题","方法"]);
  expect(((await f.gateway.get(f.published.id)).content as {fields:unknown[]}).fields).toHaveLength(2);
  await f.click(redo);expect(titles(f.host)).toHaveLength(3);
  expect(((await f.gateway.get(f.published.id)).content as {fields:unknown[]}).fields).toHaveLength(3);
});

it("serializes rapid autosaves and keeps newer input visible while an earlier save completes",async()=>{
  const f=await setup();
  const original=f.gateway.write;
  let release:()=>void=()=>{};
  const wait=new Promise<void>(resolve=>{release=resolve;});
  vi.spyOn(f.gateway,"write").mockImplementationOnce(async request=>{await wait;return original(request);});
  await f.input("模板名称","第一次名称");
  await act(async()=>f.host.querySelector('[aria-label="模板名称"]')!.dispatchEvent(new FocusEvent("focusout",{bubbles:true})));
  expect(f.host.querySelector<HTMLInputElement>('[aria-label="模板名称"]')?.disabled).toBe(false);
  await f.input("模板名称","最终名称");
  await act(async()=>f.button("添加字段").click());
  await act(async()=>release());await f.settle();
  expect(f.host.querySelector<HTMLInputElement>('[aria-label="模板名称"]')?.value).toBe("最终名称");
  expect((await f.gateway.get(f.published.id)).content).toMatchObject({name:"最终名称",fields:expect.arrayContaining([expect.objectContaining({title:"新字段"})])});
});

it("retains failed autosaves and retries the exact staged proposal without another confirmation",async()=>{
  const f=await setup();
  const write=vi.spyOn(f.gateway,"write");
  vi.spyOn(f.gateway,"write").mockRejectedValueOnce(new Error("暂时无法保存"));
  await f.click("添加字段");
  expect(titles(f.host)).toHaveLength(3);expect(f.button("重试自动保存")).toBeDefined();
  expect(((await f.gateway.get(f.published.id)).content as {fields:unknown[]}).fields).toHaveLength(2);
  await f.click("重试自动保存");expect(write).toHaveBeenCalledTimes(2);
  expect(((await f.gateway.get(f.published.id)).content as {fields:unknown[]}).fields).toHaveLength(3);
  expect(f.button("重试自动保存")).toBeUndefined();
});

it("flushes an edited template name when the inline editor closes, without publishing on read",async()=>{
  const f=await setup();
  await f.input("模板名称","关闭时自动保存");
  await dispose?.();dispose=undefined;
  await vi.waitFor(async()=>expect((await f.gateway.get(f.published.id)).content).toMatchObject({name:"关闭时自动保存"}));
});

it("durably retains an invalid autosave as a repair draft and keeps the formal template intact",async()=>{
  const f=await setup();
  await f.input("模板名称","");await f.blur("模板名称");
  const failed=await f.gateway.get(f.published.id);
  expect(failed.content).toEqual(f.published.content);
  expect(JSON.parse(failed.draft!.content).name).toBe("");
  expect(f.host.querySelector<HTMLInputElement>('[aria-label="模板名称"]')?.value).toBe("");
  await f.input("模板名称","修复后的名称");await f.blur("模板名称");
  expect((await f.gateway.get(f.published.id)).content).toMatchObject({name:"修复后的名称"});
});

it("shows the acknowledged Native validation identity beside original issues without changing the invalid draft or form history",async()=>{
  const f=await setup(),original=f.gateway.write;
  const origin={requestId:"actual-replayed-core",packageKey:"a".repeat(64),instanceId:"actual-instance",nativeRequestId:"actual-native"};
  vi.spyOn(f.gateway,"write").mockImplementation(async request=>{
    const result=await original(request);
    return result.status==="invalid"?{...result,validationOrigin:origin}:result;
  });
  await f.input("模板名称","");await f.blur("模板名称");
  const failed=await f.gateway.get(f.published.id),issue=failed.draft!.errors[0]!;
  const status=f.host.querySelector('.template-save-status')?.textContent;
  expect(status).toContain(`${issue.code}: ${issue.message} (${issue.path})`);
  expect(status).toContain(`requestId=${origin.requestId} · packageKey=${origin.packageKey} · instanceId=${origin.instanceId} · nativeRequestId=${origin.nativeRequestId}`);
  expect(failed.content).toEqual(f.published.content);expect(JSON.parse(failed.draft!.content).name).toBe("");
  expect(f.host.querySelector<HTMLInputElement>('[aria-label="模板名称"]')?.value).toBe("");
  await f.input("模板名称","修复后的名称");await f.blur("模板名称");
  expect((await f.gateway.get(f.published.id)).content).toMatchObject({name:"修复后的名称"});
  expect(f.host.querySelector('.template-save-status')).toBeNull();
  await f.click(undo);expect(f.host.querySelector<HTMLInputElement>('[aria-label="模板名称"]')?.value).toBe("");
});

it("does not expose a tampered validation origin from a typed template port",async()=>{
  const f=await setup(),original=f.gateway.write;
  vi.spyOn(f.gateway,"write").mockImplementation(async request=>{
    const result=await original(request);
    return result.status==="invalid"?{...result,validationOrigin:{requestId:request.requestId,packageKey:"a".repeat(64),
      instanceId:"/private/instance",nativeRequestId:"PRIVATE_BODY"}}:result;
  });
  await f.input("模板名称","");await f.blur("模板名称");
  const status=f.host.querySelector('.template-save-status')?.textContent;
  expect(status).toContain("requestId=");expect(status).not.toMatch(/PRIVATE|\/private|packageKey=|nativeRequestId=/u);
  expect((await f.gateway.get(f.published.id)).content).toEqual(f.published.content);
});
