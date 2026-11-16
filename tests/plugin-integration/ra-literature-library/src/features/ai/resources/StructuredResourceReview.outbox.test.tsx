// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach,expect,it,vi } from "vitest";
import type { DocumentCheckpoint, DocumentCommit, DocumentSnapshot } from "@research-assistant/core/hbs";
import type { StructuredResource } from "@research-assistant/core/structured-resources";
import type { AppHbsDocumentGateway } from "../../../../../../../apps/web/src/core/services/hbsDocumentGateway";
import type { StructuredResourceGateway } from "../../../../../../../apps/web/src/core/services/structuredResourceGateway";
import { StructuredResourceReview } from "../../../../../../../../plugins/ra-literature-library/src/features/ai/resources/StructuredResourceReview";
import { disposeStructuredHbsSessions } from "../../../../../../../../plugins/ra-literature-library/src/features/ai/resources/useStructuredHbsSession";
interface ActualHbsGateway {
  getHbsDocumentGateway():AppHbsDocumentGateway;
  getPendingHbsDocumentCommit(kind:"note"|"structured",resourceId:string):DocumentCommit<unknown>|undefined;
}
const runtime=vi.hoisted(()=>({raw:undefined as AppHbsDocumentGateway|undefined,pending:undefined as DocumentCommit<unknown>|undefined,
  actual:undefined as ActualHbsGateway|undefined,actualMode:false}));
vi.mock("../../../../../../../apps/web/src/core/services/hbsDocumentGateway",async importOriginal=>{
  const actual=await importOriginal<ActualHbsGateway>();runtime.actual=actual;
  return {...actual,getHbsDocumentGateway:()=>runtime.raw,
    getPendingHbsDocumentCommit:(kind:"note"|"structured",id:string)=>runtime.actualMode?actual.getPendingHbsDocumentCommit(kind,id):runtime.pending};
});
vi.mock("../../../../../../../apps/web/src/core/services/notifications",()=>({notify:{error:vi.fn(),success:vi.fn()}}));
const body=(text:string)=>({kind:"paper_overview_unit_v1",schemaVersion:1,fields:[{field:"q",statements:[{id:"statement",text,evidenceItemIds:[]}]}]});
it("reopens an unsettled request without replaying it, preserves new input, and settles its exact identity before accepting another submission",async()=>{
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  const baseline=body("80%"),old=body("85%"),saved=body("91%");
  let snapshot:DocumentSnapshot<unknown>={version:3,saved,baseline};
  const original:DocumentCommit<unknown>={id:"original-request",expectedVersion:2,saved,baseline};
  runtime.pending=original;
  let preserved:DocumentCheckpoint<unknown>={format:1,kind:"structured",observed:{version:2,saved:old,baseline},
    content:saved,ancestor:old,reviewAncestor:old,baselineAncestor:baseline,ordinary:[]};
  const commit=vi.fn<AppHbsDocumentGateway["commit"]>(async(_kind,_id,request)=>{
    if(request.id===original.id){runtime.pending=undefined;return {type:"receipt",requestId:request.id,snapshot:{version:3,saved,baseline}};}
    snapshot={version:snapshot.version+1,saved:request.saved,baseline:request.baseline};
    return {type:"receipt",requestId:request.id,snapshot};
  });
  runtime.raw={read:async()=>snapshot,commit,readPreservation:async()=>preserved,savePreservation:async(_kind,_id,value)=>{preserved=value as DocumentCheckpoint<unknown>;return {revision:1};}};
  const resource=():StructuredResource=>({id:"outbox",kind:"structuredReading",sourceUri:"source://library/paper/outbox",title:"阅读",revision:snapshot.version,
    content:snapshot.saved,reviewBaseline:snapshot.baseline,pendingFieldIds:["q"],reviewedFieldIds:[]});
  const review=vi.fn(),gateway={get:async()=>resource(),getSources:async()=>[],review} as unknown as StructuredResourceGateway;
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  const button=(label:string)=>[...host.querySelectorAll<HTMLButtonElement>("button")].find(value=>value.textContent===label||value.getAttribute("aria-label")===label)!;
  try{
    await act(async()=>root.render(<StructuredResourceReview resource={resource()} gateway={gateway} onChanged={()=>{}}/>));
    expect(host.textContent).toContain(original.id);expect(commit).not.toHaveBeenCalled();
    await act(async()=>button("保留第 1 处变更").click());expect(commit).not.toHaveBeenCalled();expect(review).not.toHaveBeenCalled();
    await act(async()=>button("编辑q").click());
    const input=host.querySelector<HTMLTextAreaElement>("textarea")!;
    await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")!.set!.call(input,"92%");input.dispatchEvent(new Event("input",{bubbles:true}));});
    expect(JSON.stringify(preserved.content)).toContain("92%");expect(snapshot.saved).toEqual(saved);
    await act(async()=>input.blur());expect(commit).not.toHaveBeenCalled();
    await act(async()=>button("核对原保存请求").click());
    expect(commit).toHaveBeenCalledTimes(1);expect(commit).toHaveBeenLastCalledWith("structured","outbox",original);
    expect(JSON.stringify(preserved.content)).toContain("92%");expect(snapshot.saved).toEqual(saved);
    await act(async()=>{input.focus();input.blur();});
    expect(commit).toHaveBeenCalledTimes(2);
    const next=commit.mock.calls[1]![2];expect(next.id).not.toBe(original.id);expect(next.expectedVersion).toBe(3);
    expect(snapshot.saved).toEqual(body("92%"));expect(snapshot.baseline).toEqual(baseline);
  }finally{await act(async()=>root.unmount());await disposeStructuredHbsSessions(runtime.raw);host.remove();runtime.raw=undefined;runtime.pending=undefined;vi.unstubAllGlobals();}
});

it("Escape settles an unknown request with the same identity before saving later input",async()=>{
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  let snapshot:DocumentSnapshot<unknown>={version:1,saved:body("80%"),baseline:body("80%")},preserved:DocumentCheckpoint<unknown>|undefined;
  let original:DocumentCommit<unknown>|undefined,receipt:unknown;
  runtime.pending=undefined;
  const commit=vi.fn<AppHbsDocumentGateway["commit"]>(async(_kind,_id,request)=>{
    if(original?.id===request.id)return receipt;
    snapshot={version:snapshot.version+1,saved:request.saved,baseline:request.baseline};
    const result={type:"receipt",requestId:request.id,snapshot};
    if(!original){original=request;receipt=result;throw new Error("MODEL_CONNECTION_LOST：saved, acknowledgement lost");}
    return result;
  });
  runtime.raw={read:async()=>snapshot,commit,readPreservation:async()=>preserved??null,savePreservation:async(_kind,_id,value)=>{preserved=value as DocumentCheckpoint<unknown>;return {revision:1};}};
  const resource=():StructuredResource=>({id:"active-unknown",kind:"structuredReading",sourceUri:"source://library/paper/active-unknown",title:"阅读",revision:snapshot.version,
    content:snapshot.saved,reviewBaseline:snapshot.baseline,pendingFieldIds:[],reviewedFieldIds:[]});
  const gateway={get:async()=>resource(),getSources:async()=>[]} as unknown as StructuredResourceGateway;
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  const button=(label:string)=>[...host.querySelectorAll<HTMLButtonElement>("button")].find(value=>value.textContent===label||value.getAttribute("aria-label")===label);
  const type=async(text:string)=>{const input=host.querySelector<HTMLTextAreaElement>("textarea")!;
    await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")!.set!.call(input,text);input.dispatchEvent(new Event("input",{bubbles:true}));});};
  try{
    await act(async()=>root.render(<StructuredResourceReview resource={resource()} gateway={gateway} onChanged={()=>{}}/>));
    await act(async()=>button("编辑q")!.click());await type("91%");
    await act(async()=>host.querySelector<HTMLTextAreaElement>("textarea")!.blur());
    expect(commit).toHaveBeenCalledTimes(1);expect(original).toBeDefined();
    expect(button("核对保存请求")?.dataset.requestId).toBe(original!.id);
    await act(async()=>button("编辑q")!.click());await type("93%");
    expect(JSON.stringify(preserved?.content)).toContain("93%");expect(commit).toHaveBeenCalledTimes(1);
    expect(button("核对保存请求")?.dataset.requestId).toBe(original!.id);
    await act(async()=>host.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown",{key:"Escape",bubbles:true,cancelable:true})));
    await vi.waitFor(()=>expect(snapshot.saved).toEqual(body("93%")));
    expect(commit).toHaveBeenCalledTimes(3);expect(commit.mock.calls[1]![2]).toEqual(original);
    expect(commit.mock.calls[2]![2].id).not.toBe(original!.id);
    expect(button("核对保存请求")).toBeUndefined();expect(host.querySelector("textarea")).toBeNull();
  }finally{await act(async()=>root.unmount());await disposeStructuredHbsSessions(runtime.raw);host.remove();runtime.raw=undefined;runtime.pending=undefined;vi.unstubAllGlobals();}
});

const native=vi.fn<(command:string,args?:Record<string,unknown>)=>Promise<unknown>>();
const nativeFailure={kind:"technical",phase:"validation",outcome:"unknown",pluginId:"literature_library",code:"plugin_host_exited",
  packageKey:"a".repeat(64),instanceId:"actual-instance",nativeRequestId:"actual-native-request"};
afterEach(()=>{native.mockReset();runtime.actualMode=false;runtime.raw=undefined;runtime.pending=undefined;vi.unstubAllGlobals();window.localStorage.clear();});

function resource(snapshot:DocumentSnapshot<unknown>):StructuredResource {
  return{id:"native-diagnostic",kind:"structuredReading",sourceUri:"source://library/paper/native-diagnostic",title:"阅读",revision:snapshot.version,
    content:snapshot.saved,reviewBaseline:snapshot.baseline,pendingFieldIds:[],reviewedFieldIds:[]};
}
async function mountNative(snapshot:()=>DocumentSnapshot<unknown>) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);
  runtime.actualMode=true;window.localStorage.clear();
  vi.stubGlobal("isTauri",true);vi.stubGlobal("__TAURI_INTERNALS__",{invoke:native});
  const raw=runtime.actual?.getHbsDocumentGateway();if(!raw)throw new Error("Actual HBS gateway was not loaded");
  runtime.raw=raw;
  const gateway={get:async()=>resource(snapshot()),getSources:async()=>[]} as unknown as StructuredResourceGateway;
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  await act(async()=>root.render(<StructuredResourceReview resource={resource(snapshot())} gateway={gateway} onChanged={()=>{}}/>));
  return{host,button:(label:string)=>[...host.querySelectorAll<HTMLButtonElement>("button")]
    .find(button=>button.textContent===label||button.getAttribute("aria-label")===label),
    async close(){await act(async()=>root.unmount());await disposeStructuredHbsSessions(raw);host.remove();}};
}

it.each(["hbs_read_document","hbs_read_preservation"])("shows both real identities through the original review/editor when %s fails",async failed=>{
  const snapshot={version:1,saved:body("80%"),baseline:body("80%")};
  native.mockImplementation(async command=>{
    if(command===failed)throw{code:"MODEL_STORAGE_ERROR",message:"MODEL_STORAGE_ERROR：原安全原因",requestId:"original-core-request",
      nativeFailure:{...nativeFailure,body:"PRIVATE_BODY",path:"/private/input"}};
    if(command==="hbs_read_document")return snapshot;
    if(command==="hbs_read_preservation")return null;
    if(command==="hbs_save_preservation")return{revision:1};
    throw new Error(`Unexpected command ${command}`);
  });
  const view=await mountNative(()=>snapshot);
  try{
    expect(view.host.textContent).toContain("original-core-request");expect(view.host.textContent).toContain("actual-native-request");
    expect(view.host.textContent).toContain("plugin_host_exited");expect(view.host.textContent).toContain(nativeFailure.packageKey);
    expect(view.host.textContent).not.toMatch(/PRIVATE_|\/private|\[object Object\]/u);
    if(failed==="hbs_read_preservation")expect(view.host.querySelector('[role="alert"]')?.textContent).toContain("MODEL_PRESERVATION_FAILED");
    expect(native.mock.calls.some(([command])=>command==="hbs_commit_document")).toBe(false);
  }finally{await view.close();}
});

it("shows an unstarted package failure without inventing Core or Native request IDs",async()=>{
  const snapshot={version:1,saved:body("80%"),baseline:body("80%")};
  native.mockRejectedValue({code:"MODEL_STORAGE_ERROR",message:"MODEL_STORAGE_ERROR：文献包未就绪",nativeFailure:{kind:"technical",phase:"sources",
    outcome:"notStarted",pluginId:"literature_library",code:"plugin_package_missing"}});
  const view=await mountNative(()=>snapshot);
  try{
    expect(view.host.textContent).toContain("plugin_package_missing");
    expect(view.host.textContent).not.toMatch(/requestId=|nativeRequestId=|undefined|null|\[object Object\]/u);
  }finally{await view.close();}
});

it("retains an unknown Native failure in the real outbox and retries the exact original request from the existing control",async()=>{
  let snapshot:DocumentSnapshot<unknown>={version:1,saved:body("80%"),baseline:body("80%")};
  let original:DocumentCommit<unknown>|undefined;
  native.mockImplementation(async(command,args)=>{
    if(command==="hbs_read_document")return snapshot;
    if(command==="hbs_read_preservation")return null;
    if(command==="hbs_save_preservation")return{revision:1};
    if(command==="hbs_commit_document"){
      const request=(args as {request:DocumentCommit<unknown>}).request;
      if(original===undefined){original=request;throw{code:"MODEL_INVALID_DOCUMENT",message:"MODEL_INVALID_DOCUMENT：技术失败未确认",requestId:request.id,nativeFailure};}
      snapshot={version:request.expectedVersion+1,saved:request.saved,baseline:request.baseline};
      return{type:"receipt",requestId:request.id,snapshot};
    }
    throw new Error(`Unexpected command ${command}`);
  });
  const view=await mountNative(()=>snapshot);
  try{
    await act(async()=>view.button("编辑q")?.click());
    const input=view.host.querySelector<HTMLTextAreaElement>("textarea");expect(input).not.toBeNull();
    await act(async()=>{if(!input)return;Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")?.set?.call(input,"91%");input.dispatchEvent(new Event("input",{bubbles:true}));});
    await act(async()=>input?.blur());
    await vi.waitFor(()=>expect(original).toBeDefined());
    expect(runtime.actual?.getPendingHbsDocumentCommit("structured","native-diagnostic")).toEqual(original);
    expect(view.host.textContent).toContain(original?.id);expect(view.host.textContent).toContain("actual-native-request");
    expect(view.button("核对保存请求")?.dataset.requestId).toBe(original?.id);
    await act(async()=>view.button("核对保存请求")?.click());
    expect(native.mock.calls.filter(([command])=>command==="hbs_commit_document").map(([,args])=>(args as {request:unknown}).request)).toEqual([original,original]);
    expect(snapshot.saved).toEqual(body("91%"));expect(snapshot.baseline).toEqual(body("91%"));
    expect(runtime.actual?.getPendingHbsDocumentCommit("structured","native-diagnostic")).toBeUndefined();
    expect(view.button("核对保存请求")).toBeUndefined();
  }finally{await view.close();}
});
