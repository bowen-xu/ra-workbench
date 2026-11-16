// @vitest-environment happy-dom
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach,expect,it,vi } from "vitest";
import type { StructuredResource } from "@research-assistant/core/structured-resources";
import type { AppHbsDocumentGateway } from "../../../../../../../apps/web/src/core/services/hbsDocumentGateway";
import type { StructuredResourceGateway } from "../../../../../../../apps/web/src/core/services/structuredResourceGateway";
import { StructuredResourceReview } from "../../../../../../../../plugins/ra-literature-library/src/features/ai/resources/StructuredResourceReview";
import { readingFieldColor } from "../../../../../../../../plugins/ra-literature-library/src/features/ai/readingFieldSources";
(globalThis as {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
vi.mock("../../../../../../../apps/web/src/core/services/notifications",()=>({notify:{error:vi.fn(),success:vi.fn()}}));
let dispose: (()=>Promise<void>) | undefined;
afterEach(async()=>{await dispose?.();dispose=undefined;document.body.innerHTML="";});
const pending:StructuredResource={id:"object",paperId:"paper",kind:"structuredReading",sourceUri:"source://library/paper/object",title:"阅读",revision:3,
  content:{fields:[{field:"q",statements:[{id:"statement",text:"new"}]}]},reviewBaseline:{fields:[{field:"q",statements:[{id:"statement",text:"old"}]}]},pendingFieldIds:["q"],reviewedFieldIds:[]};
async function mount(initial:StructuredResource){
  let resource=initial;
  const review=vi.fn(async()=>{resource={...resource,revision:resource.revision+1,reviewBaseline:resource.content,pendingFieldIds:[]};return resource;});
  const onChanged=vi.fn(),snapshot=()=>({version:resource.revision,saved:resource.content,baseline:resource.reviewBaseline});
  const commit=vi.fn<AppHbsDocumentGateway["commit"]>(async(_kind,_id,request)=>{
    resource={...resource,revision:resource.revision+1,content:request.saved,reviewBaseline:request.baseline};
    return {type:"receipt",requestId:request.id,snapshot:snapshot()};
  });
  const raw:AppHbsDocumentGateway={read:async()=>snapshot(),commit,readPreservation:async()=>null,savePreservation:async()=>({revision:1})};
  const gateway={get:async()=>resource,getSources:async()=>[],review} as unknown as StructuredResourceGateway;
  function Host(){const [value,setValue]=useState(initial);return <StructuredResourceReview resource={value} gateway={gateway} hbsGateway={raw} onChanged={next=>{onChanged(next);setValue(next);}}/>;}
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);dispose=()=>act(async()=>root.unmount());
  await act(async()=>root.render(<Host/>));
  return {host,review,commit,onChanged};
}
it("keeps actual card prose and sends stable HBS field decisions through the document authority",async()=>{
  const f=await mount(pending);
  expect(f.host.querySelector("del")?.textContent).toBe("old");expect(f.host.querySelector("ins")?.textContent).toBe("new");
  expect(f.host.querySelector('[data-field-id="q"] .cm-editor,textarea')).toBeNull();
  expect(f.host.querySelector('[aria-label="编辑q"]')).not.toBeNull();
  await act(async()=>f.host.querySelector<HTMLButtonElement>('button[aria-label="撤销第 1 处变更"]')!.click());
  expect(f.commit).toHaveBeenLastCalledWith("structured","object",expect.objectContaining({expectedVersion:3,saved:pending.reviewBaseline,baseline:pending.reviewBaseline}));
  expect(f.review).not.toHaveBeenCalled();expect(f.onChanged).toHaveBeenCalled();
});
it("preserves the server-owned unreviewed business scope rather than guessing protected fields",async()=>{
  const f=await mount(pending);
  await act(async()=>f.host.querySelector<HTMLButtonElement>('button[aria-label="更多阅读审阅操作"]')!.click());
  await act(async()=>[...document.querySelectorAll<HTMLButtonElement>('[role="menuitem"]')].find(value=>value.textContent==="Keep 未校验")!.click());
  expect(f.review).toHaveBeenLastCalledWith(expect.objectContaining({resourceId:"object",expectedRevision:3,action:"keep",scope:"unreviewed"}));
  expect(f.host.querySelector(".structured-card-diff")).toBeNull();
});
it("restores default card colors and explicit template colors without changing retained identities", async () => {
  const resource: StructuredResource = {
    id:"colors", kind:"structuredReading", sourceUri:"source://library/paper/colors", title:"阅读", revision:2,
    content:{paperTemplate:{fields:[{id:"question",title:"研究问题"},{id:"concepts",title:"核心概念"},{id:"findings",title:"发现",color:"#123456"}]},
      fields:["question","concepts","findings"].map(field=>({field,statements:[{id:`${field}-statement`,text:"正文"}]})),
      retainedFields:[{field:"assumptions",title:"旧前提",statements:[{id:"retained",text:"保留正文"}]}]},
    reviewBaseline:null,pendingFieldIds:[],reviewedFieldIds:[],
  };
  const {host}=await mount(resource);
  const color=(id:string)=>host.querySelector<HTMLElement>(`[data-field-id="${id}"]`)?.style.getPropertyValue("--reading-field-color");
  expect(color("question")).toBe(readingFieldColor("question"));expect(color("concepts")).toBe(readingFieldColor("concepts"));
  expect(color("question")).not.toBe(color("concepts"));expect(color("findings")).toBe("#123456");
  expect(color("$retained:assumptions")).toBe(readingFieldColor("assumptions"));
});
it("reports unavailable authority explicitly instead of falling back to the old write/review pipeline",async()=>{
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);dispose=()=>act(async()=>root.unmount());
  const raw={read:async()=>{throw new Error("MODEL_BACKEND_OFFLINE：cannot read object");}} as unknown as AppHbsDocumentGateway;
  await act(async()=>root.render(<StructuredResourceReview resource={pending} gateway={{} as StructuredResourceGateway} hbsGateway={raw} onChanged={vi.fn()}/>));
  expect(host.textContent).toContain("MODEL_OPEN_FAILED");expect(host.textContent).toContain("MODEL_BACKEND_OFFLINE");
  expect(host.querySelector(".structured-resource-cards,textarea,.cm-editor")).toBeNull();
});
