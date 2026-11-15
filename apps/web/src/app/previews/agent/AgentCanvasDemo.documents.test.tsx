/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import {AgentCanvasDemo} from "./AgentCanvasDemo";
import {createMockSystemDocuments} from "../../runtime/mock/mockSystemDocuments";
import {systemDocumentPath,systemDocumentReference} from "@research-assistant/core/agent";
import {writeComposerClipboard} from "../../../core/features/agent/agentComposerClipboard";
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
it("the canvas preview opens resolved documents in the production text tab host",async()=>{
  const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
  try{
    await act(async()=>root.render(<AgentCanvasDemo/>));
    await vi.waitFor(async()=>{await act(async()=>Promise.resolve());expect(host.querySelector('[data-action="stop"]')).not.toBeNull();});
    const sourceDocument=await createMockSystemDocuments(localStorage).gateway.read("structured-task"), path=systemDocumentPath(sourceDocument.id)!;
    const formats=new Map<string,string>(), data={getData:(type:string)=>formats.get(type)??"",setData:(type:string,value:string)=>{formats.set(type,value);}};
    writeComposerClipboard(data,{text:path,bindings:[{startUtf16:0,endUtf16:path.length,input:systemDocumentReference(sourceDocument)}]});
    const paste=new Event("paste",{bubbles:true,cancelable:true});Object.defineProperty(paste,"clipboardData",{value:data});
    await act(async()=>{host.querySelector("textarea")!.dispatchEvent(paste);});
    await act(async()=>host.querySelector<HTMLAnchorElement>("a.agent-composer-reference")!.click());
    await vi.waitFor(async()=>{await act(async()=>Promise.resolve());expect(host.querySelector(".text-editor")).not.toBeNull();});
    expect(host.querySelector('.text-documents-host[hidden]')).toBeNull();
    expect(host.querySelector('.text-editor')).not.toBeNull();
    expect(host.querySelector('[role="tablist"]')?.textContent).toContain("阅读任务提示词.md");
    expect(document.querySelector('.system-document-dialog')).toBeNull();
  }finally{await act(async()=>root.unmount());host.remove();}
});
