/** @vitest-environment happy-dom */
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, afterEach, it, expect, vi } from "vitest";
import type { AgentNoteRewriteReview } from "@research-assistant/core/agent";
import { NoteRewriteReview } from "./NoteRewriteReview";
const f=vi.hoisted(()=>({review:vi.fn(),error:vi.fn(),status:vi.fn(),applied:vi.fn()}));
vi.mock("../../../services/agentGateway",()=>({agentNoteReviewGateway:{review:f.review},agentNoteRejectionGateway:{status:f.status},agentNoteApplicationGateway:{status:f.applied}}));
vi.mock("../../../services/notifications",()=>({notify:{error:f.error}}));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
const scope={conversationId:"a1111111-1111-4111-8111-111111111111",pageId:"b1111111-1111-4111-8111-111111111111",turnId:"c1111111-1111-4111-8111-111111111111",itemId:"d1111111-1111-4111-8111-111111111111"};
function review():AgentNoteRewriteReview {
  return {schemaVersion:1,scope,currentRevision:3,baselineMatches:true,proposal:{schemaVersion:1,kind:"noteInlineRewrite",status:"proposed",
    proposalId:`note-rewrite:${"a".repeat(64)}`,conversationId:scope.conversationId,pageId:scope.pageId,turnId:scope.turnId,contextSnapshotId:"e1111111-1111-4111-8111-111111111111",
    sourceId:`note:${"b".repeat(64)}`,revision:`revision:3:sha256:${"c".repeat(64)}`,anchor:"{}",
    edits:[{blockId:"block-1",before:{kind:"inline",items:[{type:"text",text:"Original evidence",styles:[]}]},after:{kind:"inline",items:[{type:"text",text:"Reworded suggestion",styles:["bold"]}]}}],
  }};
}
let container:HTMLDivElement;let root:ReturnType<typeof createRoot>;
beforeEach(()=>{vi.clearAllMocks();container=document.createElement("div");document.body.append(container);root=createRoot(container);f.review.mockResolvedValue(review());f.status.mockResolvedValue(null);f.applied.mockResolvedValue(null);});
afterEach(async()=>{await act(async()=>root.unmount());container.remove();});
async function render(options:{saved?:boolean;revision?:number;disabled?:boolean;itemId?:string}={}) {
  await act(async()=>root.render(<StrictMode><NoteRewriteReview scope={{...scope,itemId:options.itemId??scope.itemId}} currentRevision={options.revision??3} isSaved={options.saved??true} disabled={options.disabled??false}/></StrictMode>));
}
async function click(label:string) {
  const button=[...container.querySelectorAll("button")].find(b=>b.textContent===label);
  expect(button).toBeTruthy();await act(async()=>button!.click());
}
it("does not label a durable application as unapplied or offer rejection",async()=>{
  f.applied.mockResolvedValue({schemaVersion:1,...scope,applicationId:scope.itemId,proposalId:review().proposal.proposalId,beforeRevision:3,appliedRevision:4,beforeHash:"a".repeat(64),afterHash:"b".repeat(64),createdAt:"2026-09-10T01:02:03.000Z"});
  await render({revision:4});await click("审阅改写提案");
  expect(container.textContent).toContain("已应用至版本 4");expect(container.textContent).not.toContain("未应用");
  expect([...container.querySelectorAll("button")].some(button=>button.textContent==="拒绝这条建议")).toBe(false);
});
it("reads only after explicit expansion and shows both original and proposed content without applying note edits",async()=>{
  await render();expect(f.review).not.toHaveBeenCalled();
  await click("审阅改写提案");expect(f.review).toHaveBeenCalledExactlyOnceWith(scope);
  expect(container.textContent).toContain("未应用");
  expect(container.textContent).toContain("Original evidence");expect(container.textContent).toContain("Reworded suggestion");
  expect(container.querySelectorAll("button")).toHaveLength(3);
  await render({saved:false});expect(container.textContent).toContain("不能直接应用");
  await render({revision:4});expect(container.textContent).toContain("不能直接应用");
  expect(f.review).toHaveBeenCalledTimes(1);
});
it("discards pending results on collapse or scope change",async()=>{
  let resolve!:(value:AgentNoteRewriteReview)=>void;
  f.review.mockImplementationOnce(()=>new Promise(r=>{resolve=r;}));
  await render();await click("审阅改写提案");await click("收起改写提案");
  await act(async()=>resolve(review()));expect(container.textContent).not.toContain("Original evidence");
  f.review.mockImplementationOnce(()=>new Promise(r=>{resolve=r;}));
  await click("审阅改写提案");
  const stale=resolve;
  f.review.mockImplementationOnce(()=>new Promise(()=>{}));
  await render({itemId:"ffffffff-ffff-4fff-8fff-ffffffffffff"});
  await act(async()=>stale(review()));expect(container.textContent).not.toContain("Original evidence");
});
it("offers explicit retry on native failure and disables review for unavailable metadata",async()=>{
  await render({disabled:true});await click("审阅改写提案");expect(f.review).not.toHaveBeenCalled();
  await render();f.review.mockRejectedValueOnce(new Error("unbound"));
  await click("审阅改写提案");expect(f.error).toHaveBeenCalledOnce();
  expect(container.textContent).toContain("重试核验");await click("重试核验");
  expect(container.textContent).toContain("Original evidence");
  expect(f.review).toHaveBeenCalledTimes(2);
});
