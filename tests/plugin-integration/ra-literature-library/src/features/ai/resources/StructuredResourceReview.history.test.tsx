// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import type { StructuredResource, StructuredReviewRequest, StructuredHistoryRequest } from "@research-assistant/core/structured-resources";
import { getStructuredResourceGateway } from "@research-assistant/workbench-sdk/structured-resources";
import { createMockStructuredResources } from "../../../../../../../../integration/web/src/app/runtime/mock/mockStructuredResources";
import type { AppHbsDocumentGateway } from "../../../../../../../apps/web/src/core/services/hbsDocumentGateway";
import { StructuredResourceReview } from "../../../../../../../../plugins/ra-literature-library/src/features/ai/resources/StructuredResourceReview";
(globalThis as {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
vi.mock("../../../../../../../apps/web/src/core/services/notifications",()=>({notify:{error:vi.fn(),success:vi.fn()}}));
let dispose:(()=>Promise<void>)|undefined;
afterEach(async()=>{await dispose?.();dispose=undefined;document.body.innerHTML="";vi.unstubAllGlobals();window.localStorage.clear();});
const body=(text:string)=>({kind:"paper_overview_unit_v1",schemaVersion:1,
  fields:[{field:"question",statements:[{id:"q",text,evidenceItemIds:[]}]}]});
async function setup(nativeReview=false){
  const values=new Map<string,string>();
  const storage:Storage={get length(){return values.size;},key:index=>[...values.keys()][index]??null,
    getItem:key=>values.get(key)??null,setItem:(key,value)=>{values.set(key,value);},removeItem:key=>{values.delete(key);},clear:()=>values.clear()};
  const sourceGateway={...createMockStructuredResources(storage),getSources:async()=>[]};
  const transport=nativeReview?getStructuredResourceGateway():undefined;
  const gateway={...sourceGateway,...(transport?{review:transport.review,restoreHistory:transport.restoreHistory}:{})};
  const raw:AppHbsDocumentGateway={read:(_kind,id)=>gateway.hbs.read(id),commit:(_kind,id,request)=>gateway.hbs.commit(id,request),
    readPreservation:(_kind,id)=>gateway.hbs.readPreservation(id),savePreservation:(_kind,id,value)=>gateway.hbs.savePreservation(id,value)};
  const target=(await gateway.list("paper")).find(value=>value.kind==="structuredReading")!;
  const clean=(await gateway.write({resourceId:target.id,expectedRevision:0,requestId:"initial",
    editorKind:"human",content:JSON.stringify(body("原来是解释"))})).resource;
  const changed=(await gateway.write({resourceId:target.id,expectedRevision:clean.revision,requestId:"agent",
    editorKind:"agent",content:JSON.stringify(body("现在是推理"))})).resource;
  let setResource:(next:StructuredResource)=>void=()=>{};
  function Host({initial=changed}:{initial?:StructuredResource}){const [resource,setValue]=useState(initial);setResource=setValue;
    return <StructuredResourceReview resource={resource} gateway={gateway} hbsGateway={raw} onChanged={setValue}/>;}
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  dispose=()=>act(async()=>root.unmount());
  await act(async()=>root.render(<Host/>));
  const button=(name:string)=>{const value=host.querySelector<HTMLButtonElement>(`button[aria-label="${name}"]`);
    if(!value)throw new Error(`missing ${name}`);return value;};
  const click=async(name:string)=>{await act(async()=>button(name).click());};
  const shortcut=async(redo=false)=>{await act(async()=>host.querySelector(".structured-resource-review")!.dispatchEvent(new KeyboardEvent("keydown",{key:"z",ctrlKey:true,shiftKey:redo,bubbles:true,cancelable:true})));};
  return {gateway,sourceGateway,target,host,button,click,shortcut,setResource,remount:async()=>{const latest=await gateway.get(target.id);await act(async()=>root.render(null));await act(async()=>root.render(<Host initial={latest}/>));}};
}
it("embeds word diffs, removes manual verification, and undoes/redoes Keep and manual edits with the saved baseline",async()=>{
  const f=await setup();
  const card=f.host.querySelector('[data-field-id="question"]')!;
  expect(card.querySelector("del")?.textContent).toContain("原来");
  expect(card.querySelector("ins")?.textContent).toContain("现在");
  expect(f.host.querySelector('[aria-label*="标为question人工校验"]')).toBeNull();
  expect(f.host.querySelector(".structured-resource-bulk")?.parentElement?.classList.contains("structured-resource-cards")).toBe(true);
  await f.click("保留第 1 处变更");
  expect(f.host.querySelector(".structured-card-diff")).toBeNull();
  await f.shortcut();expect(f.host.querySelector(".structured-card-diff")).not.toBeNull();
  await f.shortcut(true);expect(f.host.querySelector(".structured-card-diff")).toBeNull();
  await f.shortcut();await f.click("编辑question");
  const input=f.host.querySelector<HTMLTextAreaElement>("textarea")!;
  const revision=(await f.gateway.get(f.target.id)).revision;
  await act(async()=>input.dispatchEvent(new KeyboardEvent("keydown",{key:"z",ctrlKey:true,bubbles:true,cancelable:true})));
  expect((await f.gateway.get(f.target.id)).revision).toBe(revision);
  await act(async()=>{
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")!.set!.call(input,"人工修改");
    input.dispatchEvent(new Event("input",{bubbles:true}));
  });
  await act(async()=>input.blur());
  await vi.waitFor(()=>expect(f.host.querySelector("textarea")).toBeNull());
  expect(f.host.textContent).toContain("人工修改");
  await f.shortcut();expect(f.host.textContent).not.toContain("人工修改");
  expect(f.host.querySelector('[aria-label="修改后"]')?.textContent).toBe("现在是推理");
  await f.shortcut(true);expect(f.host.textContent).toContain("人工修改");
});
it("rebases review history over newer authority without overwriting external content or exposing a JSON metadata card",async()=>{
  const f=await setup();await f.click("保留第 1 处变更");
  expect(f.host.querySelector('[aria-label="撤销阅读操作"]')).toBeNull();
  const current=await f.gateway.get(f.target.id);
  const outside=(await f.gateway.write({resourceId:current.id,expectedRevision:current.revision,requestId:"external",
    editorKind:"agent",content:JSON.stringify({...body("外部修改"),coverage:"新来源"})})).resource;
  await act(async()=>f.setResource(outside));
  await f.shortcut();
  const saved=await f.gateway.get(f.target.id);
  expect(saved.revision).toBeGreaterThanOrEqual(outside.revision);expect(saved.content).toEqual(outside.content);
  expect(f.host.querySelector('[data-field-id="$metadata"]')).toBeNull();
  expect(f.host.textContent).not.toContain('"schemaVersion"');
});

it("shares the resource history when switching between reader and library hosts",async()=>{
  const f=await setup();await f.click("保留第 1 处变更");
  await f.remount();expect(f.host.querySelector('[aria-label="撤销阅读操作"]')).toBeNull();
  await f.shortcut();expect(f.host.querySelector(".structured-card-diff")).not.toBeNull();
});

it("orders server-owned Keep unreviewed in the same standalone shortcut history across host handoff and repeated replay",async()=>{
  const f=await setup();
  await f.click("更多阅读审阅操作");
  const keep=[...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(button=>button.textContent==="Keep 未校验")!;
  await act(async()=>keep.click());
  expect(f.host.querySelector(".structured-card-diff")).toBeNull();
  await f.remount();
  for(let index=0;index<2;index++){
    await f.shortcut();expect(f.host.querySelector(".structured-card-diff")).not.toBeNull();
    expect((await f.gateway.get(f.target.id)).content).toEqual(body("现在是推理"));
    await f.shortcut(true);expect(f.host.querySelector(".structured-card-diff")).toBeNull();
  }
  const historyFailure=f.host.querySelector('[role="status"]')?.textContent??"";
  expect(historyFailure).not.toContain("MODEL_HISTORY_FAILED");
});

const nativeFailure={kind:"technical",phase:"validation",outcome:"unknown",pluginId:"literature_library",code:"plugin_host_exited",
  packageKey:"a".repeat(64),instanceId:"actual-instance",nativeRequestId:"actual-native-request"};
function nativeBridge(){
  const invoke=vi.fn<(command:string,args?:Record<string,unknown>)=>Promise<unknown>>();
  vi.stubGlobal("isTauri",true);vi.stubGlobal("__TAURI_INTERNALS__",{invoke});
  return invoke;
}
async function keepUnreviewed(f:Awaited<ReturnType<typeof setup>>){
  await f.click("更多阅读审阅操作");
  const button=[...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(value=>value.textContent==="Keep 未校验");
  if(!button)throw new Error("missing original Keep unreviewed control");
  await act(async()=>button.click());
}
it("shows actual Native Keep diagnostics and retries its persisted request through the original control",async()=>{
  const invoke=nativeBridge(),f=await setup(true);let original:StructuredReviewRequest|undefined;
  invoke.mockImplementation(async(command,args)=>{
    if(command!=="structured_review_resource")throw new Error(`Unexpected command ${command}`);
    const request=(args as {draft:StructuredReviewRequest}).draft;
    if(!original){original=request;throw{code:"invalid_input",message:"未校验选择尚未确认",requestId:request.requestId,
      nativeFailure:{...nativeFailure,body:"PRIVATE_BODY",path:"/private/input"}};}
    return f.sourceGateway.review(request);
  });
  await keepUnreviewed(f);
  expect(original).toBeDefined();expect(f.host.textContent).toContain("MODEL_REVIEW_FAILED");
  expect(f.host.textContent).toContain(original?.requestId);expect(f.host.textContent).toContain("actual-native-request");
  expect(f.host.textContent).not.toMatch(/PRIVATE_|\/private|\[object Object\]/u);
  const key=`research-assistant:structured-request:structured_review_resource:${f.target.id}`;
  expect(JSON.parse(window.localStorage.getItem(key)!)).toEqual(original);
  expect(f.host.querySelector(".structured-card-diff")).not.toBeNull();
  await keepUnreviewed(f);
  expect(invoke.mock.calls.map(([,args])=>(args as {draft:unknown}).draft)).toEqual([original,original]);
  expect(window.localStorage.getItem(key)).toBeNull();expect(f.host.querySelector(".structured-card-diff")).toBeNull();
});

it("keeps the exact Native history request after failure and retries it before the original redo",async()=>{
  const invoke=nativeBridge(),f=await setup(true);let original:StructuredHistoryRequest|undefined;
  invoke.mockImplementation(async(command,args)=>{
    if(command==="structured_review_resource")return f.sourceGateway.review((args as {draft:StructuredReviewRequest}).draft);
    if(command!=="structured_restore_resource_history")throw new Error(`Unexpected command ${command}`);
    const request=(args as {draft:StructuredHistoryRequest}).draft;
    if(!original){original=request;throw{code:"invalid_input",message:"可信历史尚未确认",requestId:request.requestId,nativeFailure};}
    return f.sourceGateway.restoreHistory(request);
  });
  await keepUnreviewed(f);expect(f.host.querySelector(".structured-card-diff")).toBeNull();
  await f.shortcut();expect(original).toBeDefined();expect(f.host.textContent).toContain("MODEL_HISTORY_FAILED");
  expect(f.host.textContent).toContain(original?.requestId);expect(f.host.textContent).toContain("actual-native-request");
  const key=`research-assistant:structured-request:structured_restore_resource_history:${f.target.id}`;
  expect(JSON.parse(window.localStorage.getItem(key)!)).toEqual(original);
  await f.shortcut();expect(f.host.querySelector(".structured-card-diff")).not.toBeNull();
  const replays=invoke.mock.calls.filter(([command])=>command==="structured_restore_resource_history");
  expect(replays.map(([,args])=>(args as {draft:unknown}).draft)).toEqual([original,original]);expect(window.localStorage.getItem(key)).toBeNull();
  await f.shortcut(true);expect(f.host.querySelector(".structured-card-diff")).toBeNull();
});

it.each(["review","history"] as const)("shows an actual settled Core %s request identity in the original control",async operation=>{
  const invoke=nativeBridge(),f=await setup(true);let rejected:string|undefined;
  invoke.mockImplementation(async(command,args)=>{
    const request=(args as {draft:StructuredReviewRequest|StructuredHistoryRequest}).draft;
    const expected=operation==="review"?"structured_review_resource":"structured_restore_resource_history";
    if(command===expected&&!rejected){rejected=request.requestId;throw{code:"invalid_input",message:"原Core已明确拒绝",requestId:request.requestId};}
    if(command==="structured_review_resource")return f.sourceGateway.review(request as StructuredReviewRequest);
    if(command==="structured_restore_resource_history")return f.sourceGateway.restoreHistory(request as StructuredHistoryRequest);
    throw new Error(`Unexpected command ${command}`);
  });
  await keepUnreviewed(f);if(operation==="history")await f.shortcut();
  expect(rejected).toBeDefined();expect(f.host.textContent).toContain(`MODEL_${operation==="review"?"REVIEW":"HISTORY"}_FAILED`);
  expect(f.host.textContent).toContain(`requestId=${rejected}`);expect(f.host.textContent).not.toMatch(/nativeFailure|nativeRequestId|\[object Object\]/u);
  const command=operation==="review"?"structured_review_resource":"structured_restore_resource_history";
  expect(window.localStorage.getItem(`research-assistant:structured-request:${command}:${f.target.id}`)).toBeNull();
  if(operation==="review")await keepUnreviewed(f);else await f.shortcut();
  expect(f.host.querySelector(".structured-card-diff")===null).toBe(operation==="review");
});
