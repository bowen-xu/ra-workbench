/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import {createMockSystemDocuments} from "../../../app/runtime/mock/mockSystemDocuments";
import {SystemDocumentApprovalPanel} from "./SystemDocumentApprovalPanel";
import {WritingAgentPanel} from "../agent/presentation/WritingAgentPanel";
import {createMockAgentResourceGateways} from "../../../app/previews/agent/mockAgentResourceGateways";
import {createWritingDemoModelGateway} from "../../../app/previews/agent/WritingDemoModelGateway";
import {systemDocumentGateway} from "../../services/systemDocumentGateway";
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
it("the composer panel decides only the displayed request; preview cannot Keep before approval",async()=>{
 localStorage.clear();const documents=createMockSystemDocuments(localStorage),before=await documents.gateway.read("reading-reference");
 const controller=new AbortController();const writing=documents.requestWrite("chat",before.id,before.content+"\n新增说明。",controller.signal);
 for(let i=0;i<20&&!(await documents.gateway.pending("chat")).length;i++)await new Promise(resolve=>setTimeout(resolve,0));
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 const decide=vi.fn(documents.gateway.decide);
 try{
  await act(async()=>root.render(<SystemDocumentApprovalPanel conversationId="chat" gateway={{...documents.gateway,decide}}/>));
  expect(host.textContent).toContain("允许修改内置文档？");expect(host.textContent).toContain(before.path);
  expect((await documents.gateway.read(before.id)).content).toBe(before.content);
  expect(host.querySelector<HTMLButtonElement>('[aria-label="Keep all"]')?.disabled).toBe(true);
  await act(async()=>[...host.querySelectorAll('button')].find(button=>button.textContent==="允许本次写入")!.click());
  await writing;expect(decide).toHaveBeenCalledOnce();expect(host.querySelector('[aria-label="内置文档写入授权"]')).toBeNull();
  const saved=await documents.gateway.read(before.id);expect(saved.content).toContain("新增说明。");expect(saved.baseline).toBe(before.content);
 }finally{await act(async()=>root.unmount());host.remove();}
});

it("the writing composer polls its injected document gateway without native I/O",async()=>{
 const resources=createMockAgentResourceGateways(localStorage);
 const pending=vi.fn(async()=>[]);
 const native=vi.spyOn(systemDocumentGateway,"pending").mockRejectedValue(new Error("Unexpected native I/O"));
 const host=document.createElement("div");document.body.append(host);const root=createRoot(host);
 try{
  await act(async()=>root.render(<WritingAgentPanel open approvalConversationId="preview-chat"
   modelGateway={createWritingDemoModelGateway().gateway}
   resourceGateways={{...resources,systemDocuments:{...resources.systemDocuments!,pending}}}/>));
  expect(pending).toHaveBeenCalledWith("preview-chat");
  expect(native).not.toHaveBeenCalled();
 }finally{await act(async()=>root.unmount());host.remove();native.mockRestore();}
});
