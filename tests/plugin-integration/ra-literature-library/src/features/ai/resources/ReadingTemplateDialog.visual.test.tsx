// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach,expect,it,vi} from "vitest";
import {createMockStructuredResources} from "../../../../../../../../integration/web/src/app/runtime/mock/mockStructuredResources";
import {ReadingTemplateDialog} from "../../../../../../../../plugins/ra-literature-library/src/features/ai/resources/ReadingTemplateDialog";
import {notify} from "@research-assistant/workbench-sdk/feedback";
import type { AppHbsDocumentGateway } from "../../../../../../../apps/web/src/core/services/hbsDocumentGateway";
(globalThis as {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
vi.mock("../../../../../../../apps/web/src/core/services/notifications",()=>({notify:{error:vi.fn(),success:vi.fn()}}));
let dispose:(()=>Promise<void>)|undefined;
afterEach(async()=>{await dispose?.();dispose=undefined;document.body.replaceChildren();vi.restoreAllMocks();});
const body={kind:"reading_template_v1",schemaVersion:1,name:"本文模板",fields:[{id:"q",title:"问题",question:"问题定义"},{id:"m",title:"方法",question:"方法定义"}]};
async function mount(agent=false){
 const data=new Map<string,string>(),storage:Storage={get length(){return data.size;},getItem:key=>data.get(key)??null,setItem:(key,value)=>{data.set(key,value);},removeItem:key=>{data.delete(key);},clear:()=>data.clear(),key:index=>[...data.keys()][index]??null};
 const gateway=createMockStructuredResources(storage),template=(await gateway.list("paper")).find(value=>value.kind==="readingTemplate")!;
 const raw:AppHbsDocumentGateway={read:(_kind,id)=>gateway.hbs.read(id),commit:(_kind,id,request)=>gateway.hbs.commit(id,request),
   readPreservation:(_kind,id)=>gateway.hbs.readPreservation(id),savePreservation:(_kind,id,value)=>gateway.hbs.savePreservation(id,value)};
 await gateway.write({resourceId:template.id,expectedRevision:0,requestId:"initial",editorKind:agent?"agent":"human",content:JSON.stringify(body)});
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 dispose=()=>act(async()=>root.unmount());
 await act(async()=>root.render(<ReadingTemplateDialog initialResourceId={template.id} gateway={gateway} hbsGateway={raw} onClose={()=>{}}/>));
 return {gateway,template,host};
}
const button=(name:string)=>[...document.querySelectorAll<HTMLButtonElement>("button")].find(value=>value.getAttribute("aria-label")===name||value.textContent===name)!;
async function input(label:string,value:string){await act(async()=>{const element=document.querySelector<HTMLInputElement>(`[aria-label="${label}"]`)!;Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value")!.set!.call(element,value);element.dispatchEvent(new Event("input",{bubbles:true}));});}
async function settle(){await act(async()=>new Promise(resolve=>setTimeout(resolve,350)));}
it("shows object names without revision choices or publication controls",async()=>{
 await mount();
 expect(document.querySelector('select')?.textContent).not.toMatch(/· v\d/);
 expect(document.querySelector('[aria-label="选择本文阅读模板或全局模板"]')).not.toBeNull();
 expect(button("保存模板")).toBeUndefined();expect(document.body.textContent).not.toContain("确认并发布");
 expect(document.querySelector('[role="region"][aria-label="阅读模板内容"]')?.getAttribute("tabindex")).toBe("0");
});
it("autosaves human edits without creating a new Diff on clean content",async()=>{
 const {gateway,template}=await mount();const write=vi.spyOn(gateway,"write");
 await input("模板名称","新名称");expect(write).not.toHaveBeenCalled();await settle();
 expect((await gateway.get(template.id)).content).toMatchObject({name:"新名称"});
 expect((await gateway.get(template.id)).pendingFieldIds).toEqual([]);
 expect(document.querySelector('select')?.textContent).toContain("新名称");
});
it("retains Agent Diff across reopen, then Keep and Undo use the shared review gateway",async()=>{
 const {gateway,template}=await mount(true);
 expect(document.body.textContent).toContain("Keep");
 const initial=await gateway.get(template.id);expect(initial.pendingFieldIds).toContain("q");
 const kept=await gateway.review({resourceId:template.id,expectedRevision:initial.revision,requestId:"keep",action:"keep",scope:"all"});
 expect(kept.pendingFieldIds).toEqual([]);
 const updated=await gateway.write({resourceId:template.id,expectedRevision:kept.revision,requestId:"update",editorKind:"agent",content:JSON.stringify({...body,fields:[{...body.fields[0],question:"新问题"},body.fields[1]]})});
 expect(updated.resource.reviewBaseline).toEqual(body);
 const undone=await gateway.review({resourceId:template.id,expectedRevision:updated.resource.revision,requestId:"undo",action:"undo",scope:"all"});expect(undone.content).toEqual(body);
});
it("keeps failed autosaves visible and retryable",async()=>{
 const {gateway,template}=await mount();vi.spyOn(gateway,"write").mockRejectedValueOnce(new Error("测试保存失败"));
 await input("模板名称","保留输入");await settle();
 expect(document.querySelector<HTMLInputElement>('[aria-label="模板名称"]')?.value).toBe("保留输入");
 expect(notify.error).toHaveBeenCalledWith("测试保存失败");
 await act(async()=>button("重试自动保存").click());
 expect((await gateway.get(template.id)).content).toMatchObject({name:"保留输入"});
});
it("does not expose malformed repair drafts as JSON or overwrite them on view",async()=>{
 const {gateway,template}=await mount();const current=await gateway.get(template.id);
 await act(async()=>{await gateway.write({resourceId:template.id,expectedRevision:current.revision,requestId:"invalid",editorKind:"agent",content:"{invalid"});});
 const write=vi.spyOn(gateway,"write");await settle();
 expect(document.querySelector("textarea")).toBeNull();expect(document.body.textContent).not.toContain("{invalid");expect(write).not.toHaveBeenCalled();
 expect((await gateway.get(template.id)).draft?.content).toBe("{invalid");
});
