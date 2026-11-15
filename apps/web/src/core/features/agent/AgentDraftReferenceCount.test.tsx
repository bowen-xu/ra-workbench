/** @vitest-environment happy-dom */
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import type { AgentInputDraft, AgentReferenceInput } from "@research-assistant/core/agent";
import { AgentDraftReferenceCountProvider, AgentReferenceCountBadge, draftReferenceCount, useAgentDraftReferenceCount } from "./AgentDraftReferenceCount";

const file:AgentReferenceInput={type:"mention",name:"附件",target:{kind:"file",sourceId:"local",resourceId:"attachment",revision:"1"}};
const quote={type:"annotation" as const,source:"source://notes/note:#block{b}",title:"引用",text:"原文",comment:""};
const draft:AgentInputDraft={text:"@附件",bindings:[{startUtf16:0,endUtf16:3,input:file}],unboundReferences:[file],evidence:[quote,
  {...quote,text:"",image:{mediaType:"image/png",assetId:"a".repeat(64),captureId:"00000000-0000-4000-8000-000000000001",byteSize:1}}]};
it("counts reference items across annotations, images and attachments without counting an image twice",()=>{
  expect(draftReferenceCount(draft)).toBe(4);expect(draftReferenceCount({text:"普通消息",bindings:[]})).toBe(0);
});
it("publishes changes while hidden, hides zero and clears the count when its owner unmounts",async()=>{
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT",true);const host=document.createElement("div");document.body.append(host);const root=createRoot(host),publish=vi.fn();
  function Composer({value}:{value:AgentInputDraft}){const count=useAgentDraftReferenceCount(value);return <div hidden><AgentReferenceCountBadge count={count}/></div>;}
  try{
    await act(async()=>root.render(<StrictMode><AgentDraftReferenceCountProvider onChange={publish}><Composer value={draft}/></AgentDraftReferenceCountProvider></StrictMode>));
    expect(publish).toHaveBeenLastCalledWith(4);expect(host.querySelector(".agent-reference-count-badge")?.textContent).toBe("4");
    await act(async()=>root.render(<StrictMode><AgentDraftReferenceCountProvider onChange={publish}><Composer value={{text:"",bindings:[]}}/></AgentDraftReferenceCountProvider></StrictMode>));
    expect(publish).toHaveBeenLastCalledWith(0);expect(host.querySelector(".agent-reference-count-badge")).toBeNull();
    await act(async()=>root.unmount());expect(publish).toHaveBeenLastCalledWith(0);
  }finally{host.remove();vi.unstubAllGlobals();}
});
