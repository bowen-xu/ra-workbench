/** @vitest-environment happy-dom */
import {act,StrictMode} from "react";
import {createRoot} from "react-dom/client";
import {it,expect,vi,beforeEach,afterEach} from "vitest";
import {NoteRewriteDecision} from "./NoteRewriteDecision";
const f=vi.hoisted(()=>({status:vi.fn(),applied:vi.fn(),reject:vi.fn(),success:vi.fn(),error:vi.fn()}));
vi.mock("../../../services/agentGateway",()=>({agentNoteRejectionGateway:{status:f.status,reject:f.reject},agentNoteApplicationGateway:{status:f.applied}}));
vi.mock("../../../services/notifications",()=>({notify:{success:f.success,error:f.error}}));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
const scope={conversationId:"a1111111-1111-4111-8111-111111111111",pageId:"b1111111-1111-4111-8111-111111111111",turnId:"c1111111-1111-4111-8111-111111111111",itemId:"d1111111-1111-4111-8111-111111111111"};
const proposalId=`note-rewrite:${"a".repeat(64)}`;
const receipt={schemaVersion:1,...scope,decisionId:scope.itemId,proposalId,createdAt:"2026-09-10T01:02:03.000Z"};
let container:HTMLDivElement,root:ReturnType<typeof createRoot>;
beforeEach(()=>{vi.resetAllMocks();container=document.createElement("div");document.body.append(container);root=createRoot(container);f.status.mockResolvedValue(null);f.applied.mockResolvedValue(null);f.reject.mockResolvedValue(receipt);});
afterEach(async()=>{await act(async()=>root.unmount());container.remove();});
async function render(visible=true){await act(async()=>root.render(<StrictMode>{visible&&<NoteRewriteDecision scope={scope} proposalId={proposalId}/>}</StrictMode>));}
async function click(){await act(async()=>container.querySelector("button")!.click());}
it("loads persisted rejection and keeps the proposal readable without a second reject action",async()=>{
  f.status.mockResolvedValue(receipt);await render();expect(container.textContent).toContain("已拒绝");expect(container.querySelector("button")).toBeNull();expect(f.reject).not.toHaveBeenCalled();
});
it("serializes repeated clicks and reuses the request identity after an uncertain failure",async()=>{
  let reject!:(e:unknown)=>void;f.reject.mockImplementationOnce(()=>new Promise((_,r)=>{reject=r;}));
  await render();await click();await click();expect(f.reject).toHaveBeenCalledOnce();expect(container.textContent).toContain("正在记录拒绝");
  const original=f.reject.mock.calls[0]![1];await act(async()=>reject(new Error("uncertain")));
  expect(container.textContent).toContain("重试拒绝");await click();
  expect(f.reject.mock.calls[1]![1]).toBe(original);expect(container.textContent).toContain("已拒绝");expect(f.success).toHaveBeenCalledOnce();
});
it("does not turn a failed status read into a rejection",async()=>{
  f.status.mockRejectedValue(new Error("offline"));await render();expect(container.textContent).toContain("重试读取决定");expect(f.reject).not.toHaveBeenCalled();
  f.status.mockResolvedValue(null);await click();expect(container.textContent).toContain("拒绝这条建议");expect(f.reject).not.toHaveBeenCalled();
});
it("ignores late write replies after hiding and recovers durable state on reopen",async()=>{
  let resolve!:(v:unknown)=>void;f.reject.mockImplementationOnce(()=>new Promise(r=>{resolve=r;}));await render();await click();await render(false);
  await act(async()=>resolve(receipt));expect(f.success).not.toHaveBeenCalled();expect(container.textContent).toBe("");
  f.status.mockResolvedValue(receipt);await render();expect(container.textContent).toContain("已拒绝");expect(f.reject).toHaveBeenCalledOnce();
});
it("discards an old scope's status response after switching the target",async()=>{
  const replies:Array<(v:unknown)=>void>=[];
  f.status.mockImplementation(()=>new Promise(r=>replies.push(r)));await render();
  f.status.mockResolvedValue(null);
  await act(async()=>root.render(<StrictMode><NoteRewriteDecision scope={{...scope,itemId:scope.pageId}} proposalId={proposalId}/></StrictMode>));
  await act(async()=>replies.forEach(resolve=>resolve(receipt)));
  expect(container.textContent).toContain("拒绝这条建议");expect(container.textContent).not.toContain("已拒绝");
});

const application={schemaVersion:1,...scope,applicationId:scope.itemId,proposalId,beforeRevision:3,appliedRevision:4,beforeHash:"a".repeat(64),afterHash:"b".repeat(64),createdAt:receipt.createdAt};
it("restores an application without a reject action or an unapplied claim",async()=>{
  f.applied.mockResolvedValue(application);await render();
  expect(container.textContent).toContain("已应用至版本 4");expect(container.textContent).toContain("当前正文可能已有后续修改");
  expect(container.textContent).not.toContain("未应用");expect(container.querySelector("button")).toBeNull();expect(f.reject).not.toHaveBeenCalled();
});
it("requires both decision reads to succeed and rejects inconsistent history",async()=>{
  f.applied.mockRejectedValue(new Error("offline"));await render();
  expect(container.textContent).toContain("重试读取决定");expect(container.textContent).not.toContain("未应用");
  f.applied.mockResolvedValue(application);f.status.mockResolvedValue(receipt);await click();
  expect(container.textContent).toContain("重试读取决定");expect(f.reject).not.toHaveBeenCalled();
});
it("ignores old application replies after changing the target",async()=>{
  const replies:Array<(v:unknown)=>void>=[];f.applied.mockImplementation(()=>new Promise(r=>replies.push(r)));await render();
  f.applied.mockResolvedValue(null);
  await act(async()=>root.render(<StrictMode><NoteRewriteDecision scope={{...scope,itemId:scope.pageId}} proposalId={proposalId}/></StrictMode>));
  await act(async()=>replies.forEach(resolve=>resolve(application)));
  expect(container.textContent).toContain("未应用");expect(container.textContent).not.toContain("已应用至");
});
it("refreshes a concurrently applied decision without retrying the rejection",async()=>{
  await render();f.reject.mockRejectedValue("decided");f.applied.mockResolvedValue(application);
  await click();expect(container.textContent).toContain("已应用至版本 4");
  expect(f.reject).toHaveBeenCalledOnce();expect(container.querySelector("button")).toBeNull();
});

it("accepts only once and refreshes durable decisions after the workspace finishes",async()=>{
  let finish!:()=>void;
  const accept=vi.fn(()=>new Promise<void>(resolve=>{finish=resolve;}));
  await act(async()=>root.render(<NoteRewriteDecision scope={scope} proposalId={proposalId} onAccept={accept}/>));
  const button=[...container.querySelectorAll("button")].find(b=>b.textContent==="接受这条建议")!;
  await act(async()=>{button.click();button.click();});
  expect(accept).toHaveBeenCalledOnce();expect(container.textContent).toContain("正在应用并核对");
  expect([...container.querySelectorAll("button")].every(b=>b.disabled)).toBe(true);
  f.applied.mockResolvedValue(application);
  await act(async()=>finish());
  expect(container.textContent).toContain("已应用至版本 4");expect(f.reject).not.toHaveBeenCalled();
});

it("disables acceptance on a stale review and all mutations while another operation needs recovery",async()=>{
  const accept=vi.fn();
  await act(async()=>root.render(<NoteRewriteDecision scope={scope} proposalId={proposalId} onAccept={accept} acceptDisabled/>));
  const buttons=[...container.querySelectorAll("button")];
  expect(buttons.find(b=>b.textContent==="接受这条建议")?.disabled).toBe(true);
  expect(buttons.find(b=>b.textContent==="拒绝这条建议")?.disabled).toBe(false);
  await act(async()=>root.render(<NoteRewriteDecision scope={scope} proposalId={proposalId} onAccept={accept} operationBlocked/>));
  expect([...container.querySelectorAll("button")].every(b=>b.disabled)).toBe(true);
  expect(container.textContent).toContain("待核对的修改操作");
  expect(container.textContent).not.toContain("未应用");
});

it("does not interpret a lost acceptance reply as unapplied and survives hiding the review",async()=>{
  let fail!:(reason:unknown)=>void;
  const accept=vi.fn(()=>new Promise<void>((_resolve,reject)=>{fail=reject;}));
  await act(async()=>root.render(<NoteRewriteDecision scope={scope} proposalId={proposalId} onAccept={accept}/>));
  await act(async()=>container.querySelector("button")!.click());
  await act(async()=>root.render(<div/>));
  await act(async()=>fail(new Error("reply lost")));
  f.applied.mockResolvedValue(application);
  await act(async()=>root.render(<NoteRewriteDecision scope={scope} proposalId={proposalId} onAccept={accept}/>));
  expect(container.textContent).toContain("已应用至版本 4");expect(accept).toHaveBeenCalledOnce();
});
