/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach,expect,it,vi} from "vitest";
import {AgentReferenceDocument} from "./AgentReferenceDocument";
import {createMockAgentResourceGateways} from "../../../app/previews/agent/mockAgentResourceGateways";
import {createMockSystemDocuments} from "../../../app/runtime/mock/mockSystemDocuments";
import {OPEN_TEXT_DOCUMENT,type TextDocumentRequest} from "../../components/text-editor/textDocumentEvents";
import {systemDocumentGateway} from "../../services/systemDocumentGateway";
import {notify} from "../../services/notifications";
import type {AgentReferenceInput,SystemDocument,SystemDocumentGateway} from "@research-assistant/core/agent";
vi.mock("@tauri-apps/api/core",()=>({isTauri:()=>false,invoke:vi.fn(()=>{throw new Error("unexpected native IPC");})}));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
afterEach(()=>{document.body.replaceChildren();vi.restoreAllMocks();});
const reference:AgentReferenceInput={type:"mention",name:"阅读任务提示词",target:{kind:"file",sourceId:"builtin:structured-task",resourceId:"structured-task"}};
async function mount(target:AgentReferenceInput=reference,gateway?:SystemDocumentGateway){
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host),close=vi.fn();
  const resources=gateway?{...createMockAgentResourceGateways(),systemDocuments:gateway}:undefined;
  await act(async()=>root.render(<AgentReferenceDocument reference={target} onClose={close} {...(resources?{gateways:resources}:{})}/>));
  return {root,close,dispose:()=>act(async()=>root.unmount())};
}
it("does not open any panel or tab before resolution, then opens only the exact current document",async()=>{
  const gateway=createMockSystemDocuments(localStorage).gateway;
  let resolve!:(value:SystemDocument)=>void;
  const read=vi.fn(()=>new Promise<SystemDocument>(done=>{resolve=done;})),list=vi.fn(gateway.list);
  let opened:TextDocumentRequest|undefined;
  const listener=(event:Event)=>{event.preventDefault();opened=(event as CustomEvent<TextDocumentRequest>).detail;};
  window.addEventListener(OPEN_TEXT_DOCUMENT,listener);
  const view=await mount(reference,{...gateway,read,list});
  try{
    expect(read).toHaveBeenCalledExactlyOnceWith("structured-task");expect(list).not.toHaveBeenCalled();
    expect(opened).toBeUndefined();expect(document.querySelector('[role="dialog"],dialog')).toBeNull();
    await act(async()=>resolve(await gateway.read("structured-task")));
    expect(opened?.id).toBe("system:structured-task");expect(opened?.path).toBe("source://agents/prompts/structured-task.md");
    expect(opened?.content.length).toBeGreaterThan(0);expect(view.close).toHaveBeenCalledOnce();
  }finally{window.removeEventListener(OPEN_TEXT_DOCUMENT,listener);await view.dispose();}
});
it.each(["file","skill"])("a missing builtin %s reports the common error without opening UI",async kind=>{
  const gateway=createMockSystemDocuments(localStorage).gateway,error=vi.spyOn(notify,"error"),read=vi.fn().mockRejectedValue(new TypeError("Cannot read properties of undefined (reading 'invoke')"));
  const target:AgentReferenceInput=kind==="file"?reference:{type:"skill",name:"arbitrary display name",identity:{environment_id:"application",source_id:".agents/skills/edit-structured-resources/SKILL.md",revision:"legacy"}};
  const view=await mount(target,{...gateway,read});
  try{expect(error).toHaveBeenCalledExactlyOnceWith("找不到文件对象");expect(document.querySelector('[role="dialog"],dialog')).toBeNull();expect(view.close).toHaveBeenCalledOnce();}
  finally{await view.dispose();}
});
it("bare browser runtime never calls native IPC for a system document",async()=>{
  const {invoke}=await import("@tauri-apps/api/core");
  await expect(systemDocumentGateway.read("structured-task")).rejects.toThrow("找不到文件对象");
  expect(invoke).not.toHaveBeenCalled();
});
it("a request completed after unmount never opens a late document",async()=>{
  const gateway=createMockSystemDocuments(localStorage).gateway;
  let resolve!:(value:SystemDocument)=>void;
  const event=vi.fn();window.addEventListener(OPEN_TEXT_DOCUMENT,event);
  const view=await mount(reference,{...gateway,read:()=>new Promise(done=>{resolve=done;})});
  await view.dispose();
  try{await act(async()=>resolve(await gateway.read("structured-task")));expect(event).not.toHaveBeenCalled();}
  finally{window.removeEventListener(OPEN_TEXT_DOCUMENT,event);}
});

it("unknown skill paths use the same missing-object feedback, never a skill dialog",async()=>{
  const error=vi.spyOn(notify,"error"), read=vi.fn().mockRejectedValue(new Error("private diagnostic"));
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host),close=vi.fn();
  try{
    await act(async()=>root.render(<AgentReferenceDocument reference={{type:"skill",name:"missing",identity:{environment_id:"application",source_id:".agents/skills/missing/SKILL.md",revision:"legacy"}}} onClose={close} gateways={{...createMockAgentResourceGateways(),readSkillDocument:read}}/>));
    expect(error).toHaveBeenCalledExactlyOnceWith("找不到文件对象");expect(document.querySelector('[role="dialog"],dialog')).toBeNull();expect(close).toHaveBeenCalledOnce();
  }finally{await act(async()=>root.unmount());}
});

it("accepts only canonical document addresses and rejects legacy or mismatched assets",async()=>{
  const {decodeSystemDocument}=await import("../../services/systemDocumentGateway");
  const document={id:"structured-task",name:"任务",workflow:"shared",content:"text",baseline:"text",revision:"storage-only"};
  const path="source://agents/prompts/structured-task.md";
  expect(()=>decodeSystemDocument({...document,path:"source://system/structured-task"})).toThrow("内置文档身份无效");
  expect(decodeSystemDocument({...document,path}).path).toBe(path);
  expect(()=>decodeSystemDocument({...document,path:"source://agents/skills/missing/SKILL.md"})).toThrow("内置文档身份无效");
});
