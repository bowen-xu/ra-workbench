/** @vitest-environment happy-dom */
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { noteBlockId, notePageId } from "@research-assistant/core/notes";
import type { NoteDocumentSnapshot } from "@research-assistant/data/notes";
import type { AgentNotePrepareRequest, AgentConversationProjection, AgentSearchHit, AgentItemProjection } from "@research-assistant/core/agent";
import { NoteAgentPanel } from "./NoteAgentPanel";
import { AgentDraftProvider } from "../../agent/AgentDraftProvider";
import { resetNoteSubmissionOwnersForTests } from "../../agent/noteSubmissionController";
import { NoteQueueSubmissionController, resetNoteQueueSubmissionOwnersForTests } from "../../agent/noteQueueSubmissionController";
import type { NoteSaveStatus } from "../editor/noteSaveController";
import * as commandPort from "../../agent/agentCommandGateway";
import { agentCreationGateway, agentMetadataGateway } from "../../../services/agentGateway";
import { creationProof } from "../../agent/conversationCreationTestFacts";
import { getAppStorage } from "../../../runtime/appStorage";
import { resetConversationCreationOwnersForTests } from "../../agent/conversationCreationController";

const f = vi.hoisted(()=>({steer:vi.fn(),steerRecovery:vi.fn(),queueRead:vi.fn(),queueEdit:vi.fn(),queuePrepare:vi.fn(),queueRecover:vi.fn(),queueCancel:vi.fn(),queueStart:vi.fn(),queueRestore:vi.fn(),queueList:vi.fn(),prepare:vi.fn(),restore:vi.fn(),start:vi.fn(),interrupt:vi.fn(),refresh:vi.fn(),recovery:vi.fn(),review:vi.fn(),status:"ready" as "ready"|"syncing",projection:null as AgentConversationProjection|null,hit:undefined as AgentSearchHit|undefined}));
const conversationId = "a1111111-1111-4111-8111-111111111111";
const pageId = notePageId("b1111111-1111-4111-8111-111111111111");
const turnId = "c1111111-1111-4111-8111-111111111111";
vi.mock("../../../services/agentGateway",async importOriginal=>({...(await importOriginal<typeof import("../../../services/agentGateway")>()),agentNoteSteeringGateway:{steer:f.steer,recover:f.steerRecovery},agentNoteQueueGateway:{read:f.queueRead,edit:f.queueEdit,prepare:f.queuePrepare,recover:f.queueRecover,cancel:f.queueCancel,start:f.queueStart,restore:f.queueRestore,list:f.queueList},agentNoteSubmissionGateway:{prepare:f.prepare,restore:f.restore,start:f.start},agentSubtaskGateway:{list:vi.fn(async()=>[])},agentTurnGateway:{interrupt:f.interrupt},agentNoteRecoveryGateway:{list:f.recovery},agentNoteReviewGateway:{review:f.review}}));
vi.mock("../../agent/BoundConversationList",()=>({BoundConversationList:({onSelect}:{onSelect:(id:string,archived?:boolean,hit?:AgentSearchHit,pending?:boolean)=>void})=><><button onClick={()=>onSelect("a1111111-1111-4111-8111-111111111111",false,f.hit)}>选择测试对话</button><button onClick={()=>onSelect("a2222222-2222-4222-8222-222222222222",false)}>切换其它对话</button><button onClick={()=>onSelect("a1111111-1111-4111-8111-111111111111",false,undefined,true)}>选择本地新对话</button></>}));
vi.mock("../../agent/useAgentConversation",()=>({useAgentConversation:()=>({status:f.status,projection:f.projection})}));
vi.mock("../../agent/useConversationMetadata",()=>({useConversationMetadata:()=>({status:"ready",conversation:{archived:false},refresh:f.refresh})}));
vi.mock("../../agent/AgentModelControl",()=>({AgentModelControl:()=> <span>模型选择</span>}));
vi.mock("../../speech/PaperAiReadingSpeechInput",()=>({PaperAiReadingSpeechInput:()=> <span>语音输入</span>}));
vi.mock("../../../services/notifications",()=>({notify:{error:vi.fn(),info:vi.fn()}}));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;

let container:HTMLDivElement;
let root:ReturnType<typeof createRoot>;
function snapshot(revision=3):NoteDocumentSnapshot {
  return {page:{id:pageId,title:"Synthetic note",position:0,status:"active",isFavorite:false,revision,schemaVersion:1,projectIds:[],tagIds:[],createdAt:"now",updatedAt:"now"},
    document:{schemaVersion:1,pageId,blocks:[{id:noteBlockId("block-1"),type:"paragraph",props:{},content:{kind:"inline",items:[{type:"text",text:"Saved evidence",styles:[]}]},children:[]}]}};
}
async function render(saveStatus:NoteSaveStatus="saved",revision=3,autoDispatch=false) {
  await act(async()=>root.render(<StrictMode><AgentDraftProvider><NoteAgentPanel snapshot={snapshot(revision)} saveStatus={saveStatus} autoDispatch={autoDispatch}/></AgentDraftProvider></StrictMode>));
}
function button(label:string):HTMLButtonElement {
  const found=Array.from(container.querySelectorAll("button")).find(el=>el.getAttribute("aria-label")===label || el.textContent===label);
  if(!found) throw new Error(`Missing button: ${label}`);return found;
}
async function click(label:string) {await act(async()=>button(label).click());}
async function type(value:string) {
  const input=container.querySelector("textarea")!;
  await act(async()=>{
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")!.set!.call(input,value);
    input.dispatchEvent(new Event("input",{bubbles:true}));
  });
}
beforeEach(()=>{
  resetConversationCreationOwnersForTests(getAppStorage());getAppStorage().clear();
  resetNoteSubmissionOwnersForTests(); resetNoteQueueSubmissionOwnersForTests();
  vi.clearAllMocks();f.steer.mockReset();f.steerRecovery.mockReset();f.status="ready";f.projection=null;f.hit=undefined;
  f.queueRead.mockImplementation(async scope=>({...scope,schemaVersion:1,revision:0,paused:false,preparingInputId:null,inputs:[]}));
  f.queueList.mockImplementation(async scope=>({schemaVersion:1,conversationId:scope.conversationId,scope:scope.scope,items:[],nextCursor:null}));
  container=document.createElement("div");document.body.append(container);root=createRoot(container);
  f.prepare.mockImplementation(async(request:AgentNotePrepareRequest)=>({schemaVersion:1,conversationId,pageId,requestId:request.requestId,turnId,contextSnapshotId:"d1111111-1111-4111-8111-111111111111",input:{text:request.text,model:{providerId:"deepseek",modelId:"deepseek-v4-pro",revision:null},reasoningEffort:null,context:[{kind:"note",sourceId:"note:synthetic",resourceId:pageId,revision:`revision:${request.context.expectedRevision}`,text:"Frozen evidence",anchor:null}]}}));
  f.start.mockImplementation(async preview=>({schemaVersion:1,conversationId,requestId:preview.requestId,turnId,alreadyStarted:false,outcome:null}));
  f.interrupt.mockResolvedValue({schemaVersion:1,conversationId,turnId,cancellationRequested:true});
});
afterEach(async()=>{await act(async()=>root.unmount());container.remove();});

it("keeps the draft and textarea instance when hiding the selected session",async()=>{
  await render();await click("打开 AI 笔记助手");await click("选择测试对话");
  await type("Question draft");
  const input=container.querySelector("textarea");
  await click("隐藏 AI 笔记助手");
  expect(container.querySelector("section")?.hidden).toBe(true);
  expect(document.activeElement).toBe(button("打开 AI 笔记助手"));
  await click("打开 AI 笔记助手");
  expect(container.querySelector("textarea")).toBe(input);
  expect(input?.value).toBe("Question draft");expect(f.prepare).not.toHaveBeenCalled();
  expect(f.interrupt).not.toHaveBeenCalled();
});

it("releases a resolved failed creation promise so an explicit retry is not permanently suppressed",async()=>{
  const create=vi.spyOn(agentCreationGateway,"createBound").mockRejectedValue("notFound");
  try {
    await render();await click("打开 AI 笔记助手");await click("选择本地新对话");await type("Retained creation draft");
    await click("准备发送消息");await click("准备发送消息");
    expect(create).toHaveBeenCalledTimes(2);expect(create.mock.calls[0]).toEqual(create.mock.calls[1]);
    expect(f.prepare).not.toHaveBeenCalled();expect(f.start).not.toHaveBeenCalled();
    expect(container.querySelector("textarea")?.value).toBe("Retained creation draft");
  } finally {create.mockRestore();}
});

it("keeps a note creation Unknown on its original intent and adopts only after explicit status then current read",async()=>{
  const create=vi.spyOn(agentCreationGateway,"createBound").mockRejectedValueOnce(new Error("lost reply"));
  const status=vi.spyOn(agentCreationGateway,"createBoundStatus").mockImplementation(async intent=>({schemaVersion:1,operationId:intent.operationId,conversationId:intent.id,status:{type:"committed",proof:creationProof(intent)}}));
  const read=vi.spyOn(agentMetadataGateway,"read").mockImplementation(async request=>({id:request.id,title:"Current note",archived:false,durableCount:0,createdAt:"now",updatedAt:"now"}));
  try {
    await render();await click("打开 AI 笔记助手");await click("选择本地新对话");await type("Retained note question");await click("准备发送消息");
    const intent=create.mock.calls[0]![0];await click("准备发送消息");expect(create).toHaveBeenCalledTimes(1);
    await click("核对原创建操作");expect(status).toHaveBeenCalledExactlyOnceWith(intent);expect(read).not.toHaveBeenCalled();
    await click("读取当前对话");expect(read).toHaveBeenCalledExactlyOnceWith({id:conversationId,binding:{kind:"note",resourceId:pageId}});
    expect(f.prepare).not.toHaveBeenCalled();expect(f.start).not.toHaveBeenCalled();expect(container.querySelector("textarea")?.value).toBe("Retained note question");
    await click("准备发送消息");expect(f.prepare).toHaveBeenCalledOnce();expect(create).toHaveBeenCalledTimes(1);
  } finally {create.mockRestore();status.mockRestore();read.mockRestore();}
});

it("reattaches a switched conversation to its pending note preview",async()=>{
  let resolve!:(value:unknown)=>void;
  f.prepare.mockImplementationOnce(()=>new Promise(done=>{resolve=done;}));
  await render();await click("打开 AI 笔记助手");await click("选择测试对话");await type("Keep this prepared note");await click("准备发送消息");
  const prepared=f.prepare.mock.calls[0]?.[0] as AgentNotePrepareRequest;
  await click("切换其它对话");await click("选择测试对话");
  await act(async()=>{resolve({schemaVersion:1,conversationId,pageId,requestId:prepared.requestId,turnId,contextSnapshotId:"d1111111-1111-4111-8111-111111111111",input:{text:prepared.text,model:{providerId:"deepseek",modelId:"deepseek-v4-pro",revision:null},reasoningEffort:null,context:[]}});await Promise.resolve();});
  expect(Array.from(container.querySelectorAll("button")).some(el=>el.textContent==="确认发送")).toBe(true);
});

it("automatically dispatches the default note submission after freezing its context",async()=>{
  await render("saved",4,true);await click("打开 AI 笔记助手");await click("选择测试对话");await type("Explain this block");
  await click("准备发送消息");
  await act(async()=>{await Promise.resolve();await Promise.resolve();});
  expect(f.start).toHaveBeenCalledWith(expect.objectContaining({input:expect.objectContaining({text:"Explain this block"})}),"deepSeekNonThinking");
  expect(Array.from(container.querySelectorAll("button")).some(el=>el.textContent==="确认发送")).toBe(false);
});

it("waits for saved revision, reviews the selected block and sends only after confirmation",async()=>{
  await render("dirty");await click("打开 AI 笔记助手");await click("选择测试对话");await type("Explain this block");
  expect(button("准备发送消息").disabled).toBe(true);
  await render("saved",4);
  const scope=container.querySelector<HTMLSelectElement>('select[aria-label="笔记来源范围"]')!;
  await act(async()=>{scope.value="block-1";scope.dispatchEvent(new Event("change",{bubbles:true}));});
  await click("准备发送消息");
  expect(f.prepare).toHaveBeenCalledWith(expect.objectContaining({conversationId,text:"Explain this block",context:{pageId,expectedRevision:4,blockId:"block-1"}}));
  expect(f.start).not.toHaveBeenCalled();
  expect(container.textContent).toContain("Frozen evidence");
  await type("Next draft");await render("saved",5);
  await click("确认发送");
  expect(f.start).toHaveBeenCalledWith(expect.objectContaining({input:expect.objectContaining({text:"Explain this block",context:[expect.objectContaining({revision:"revision:4"})]})}),"deepSeekNonThinking");
  expect(container.querySelector("textarea")?.value).toBe("Next draft");
  await click("停止生成");await click("停止生成");
  expect(f.interrupt).toHaveBeenCalledOnce();
  expect(container.textContent).toContain("最终状态以对话历史为准");
});

it("returns to editable input after a definitive first rejection without losing the draft",async()=>{
  await render();await click("打开 AI 笔记助手");await click("选择测试对话");await type("Explain");
  await click("准备发送消息");f.start.mockRejectedValueOnce({code:"unsupportedModel"});
  await click("确认发送");expect(container.textContent).toContain("本次发送已被拒绝");
  await click("返回输入");expect(container.querySelector("textarea")?.value).toBe("Explain");
  expect(button("准备发送消息").disabled).toBe(false);
});

it("keeps an uncertain start on the same receipt and never sends on IME Enter",async()=>{
  await render();await click("打开 AI 笔记助手");await click("选择测试对话");await type("Explain");
  await act(async()=>container.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",isComposing:true,bubbles:true})));
  expect(f.prepare).not.toHaveBeenCalled();
  await click("准备发送消息");f.start.mockRejectedValueOnce(new Error("offline"));
  await click("确认发送");expect(container.textContent).not.toContain("返回输入");
  await click("查询或继续原轮次");expect(f.start.mock.calls[0]).toEqual(f.start.mock.calls[1]);
  expect(f.prepare).toHaveBeenCalledOnce();
});

function history(outcome:"completed"|null):AgentConversationProjection {
  return {conversationId,seq:8,activeTurn:outcome?null:turnId,turnOrder:[turnId],turns:new Map([[turnId,{
    contextSnapshotId:"snapshot",model:{providerId:"deepseek",modelId:"deepseek-v4-pro",revision:null},phase:"runningModel",outcome,
    itemOrder:["message"],items:new Map([["message",{kind:{type:"assistantMessage"},text:"Synthetic answer",completed:!!outcome,interrupted:false}]]),
    attemptOrder:[],attempts:new Map(),inputTokens:null,outputTokens:null,
  }]])};
}

it("offers native rewrite review only for a completed successful result paired with its rewrite tool",async()=>{
  const itemId="d1111111-1111-4111-8111-111111111111";
  const result:AgentItemProjection={kind:{type:"toolResult",callId:"call-1",isError:false},text:'{"kind":"noteInlineRewrite"}',completed:true,interrupted:false};
  function project(item:AgentItemProjection,toolName="research_propose_note_rewrite") {
    const base=history("completed");const turn=base.turns.get(turnId)!;
    f.projection={...base,turns:new Map([[turnId,{...turn,itemOrder:["call",itemId],items:new Map<string,AgentItemProjection>([
      ["call",{kind:{type:"toolCall",callId:"call-1",name:toolName},text:"{}",completed:true,interrupted:false}], [itemId,item],
    ])}]])};
  }
  await render();await click("打开 AI 笔记助手");await click("选择测试对话");
  for(const invalid of [{...result,kind:{type:"assistantMessage"} as const},{...result,completed:false},{...result,interrupted:true},
    {...result,kind:{type:"toolResult",callId:"call-1",isError:true} as const}]) {
    project(invalid);await render();expect(container.textContent).not.toContain("审阅改写提案");
  }
  project(result,"research_read_source");await render();expect(container.textContent).not.toContain("审阅改写提案");
  project(result);await render();expect(f.review).not.toHaveBeenCalled();
  expect(container.querySelectorAll(".note-agent-activity-group")).toHaveLength(1);
  expect(container.querySelector(".note-agent-activity-group summary")?.textContent).toContain("research_propose_note_rewrite");
  f.review.mockImplementationOnce(()=>new Promise(()=>{}));await click("审阅改写提案");
  expect(f.review).toHaveBeenCalledExactlyOnceWith({conversationId,pageId,turnId,itemId});
  expect(f.start).not.toHaveBeenCalled();expect(f.prepare).not.toHaveBeenCalled();
});

it("stops a reopened active turn once and releases its spinner on the durable outcome",async()=>{
  f.projection=history(null);
  await render();await click("打开 AI 笔记助手");await click("选择测试对话");
  await click("停止生成");await click("停止生成");expect(f.interrupt).toHaveBeenCalledOnce();
  f.projection=history("completed");await render();
  expect(container.textContent).not.toContain("正在等待任务停止");
  expect(container.textContent).toContain("Synthetic answer");
});

it("uses a terminal history snapshot that arrives before the admission reply",async()=>{
  await render();await click("打开 AI 笔记助手");await click("选择测试对话");await type("Explain");
  await click("准备发送消息");
  f.projection=history("completed");await render();
  await click("确认发送");
  expect(container.textContent).toContain("本轮已完成");
  expect(Array.from(container.querySelectorAll("button")).some(el=>el.getAttribute("aria-label")==="停止生成")).toBe(false);
});

it("waits for synchronized history before consuming a retained terminal outcome",async()=>{
  await render();await click("打开 AI 笔记助手");await click("选择测试对话");await type("Explain");
  await click("准备发送消息");await click("确认发送");
  f.status="syncing";f.projection=history("completed");await render();
  expect(container.textContent).not.toContain("本轮已完成");
  f.status="ready";await render();
  expect(container.textContent).toContain("本轮已完成");
});

it("focuses the searched message using its turn and item identities",async()=>{
  f.projection=history("completed");
  f.hit={conversation:{id:conversationId,title:"Synthetic",archived:false} as AgentSearchHit["conversation"],excerpt:"answer",itemId:"message",turnId,startSeq:2,completed:true};
  const scroll=vi.spyOn(HTMLElement.prototype,"scrollIntoView").mockImplementation(()=>{});
  try {
    await render();await click("打开 AI 笔记助手");await click("选择测试对话");
    expect(document.activeElement).toBe(container.querySelector('article[data-item-id="message"]'));
    expect(scroll).toHaveBeenCalledWith({block:"center"});
  } finally {scroll.mockRestore();}
});

it("discovers and reviews an old submission without overwriting the current draft or sending",async()=>{
  const requestId="e1111111-1111-4111-8111-111111111111";
  f.recovery.mockResolvedValue({schemaVersion:1,conversationId,pageId,nextCursor:null,items:[{requestId,turnId,messageExcerpt:"Original question",createdAt:"2026-09-10T12:00:00Z"}]});
  f.restore.mockResolvedValue({schemaVersion:1,conversationId,pageId,requestId,turnId,contextSnapshotId:"old-snapshot",input:{text:"Original question",model:{providerId:"deepseek",modelId:"deepseek-v4-pro",revision:null},reasoningEffort:null,context:[{text:"Original evidence",revision:"revision:1"}]}});
  await render("dirty",9);await click("打开 AI 笔记助手");await click("选择测试对话");await type("New unsaved question");
  expect(f.recovery).not.toHaveBeenCalled();
  await click("检查未启动的提交");
  expect(f.recovery).toHaveBeenCalledWith({conversationId,pageId,after:null,limit:20});
  await click("查看原提交");
  expect(f.restore).toHaveBeenCalledWith({conversationId,pageId,requestId});
  expect(f.prepare).not.toHaveBeenCalled();expect(f.start).not.toHaveBeenCalled();
  expect(container.textContent).toContain("Original evidence");
  await click("确认发送");
  expect(container.querySelector("textarea")?.value).toBe("New unsaved question");
  expect(f.start.mock.calls[0]?.[0].input.text).toBe("Original question");
});

function queueFixture(){
  let queue={schemaVersion:1 as const,conversationId,scope:{kind:"note" as const,resourceId:pageId},revision:0,paused:false,preparingInputId:null as string|null,inputs:[] as {inputId:string;text:string}[]};
  f.queueRead.mockImplementation(async()=>structuredClone(queue));
  f.queueEdit.mockImplementation(async request=>{
    const edit=request.edit;
    if(edit.type==="add")queue={...queue,revision:queue.revision+1,inputs:[...queue.inputs,{inputId:edit.inputId,text:edit.text}]};
    else if(edit.type==="update")queue={...queue,revision:queue.revision+1,inputs:queue.inputs.map(i=>i.inputId===edit.inputId?{...i,text:edit.text}:i)};
    else if(edit.type==="remove")queue={...queue,revision:queue.revision+1,inputs:queue.inputs.filter(i=>i.inputId!==edit.inputId)};
    else if(edit.type==="reorder")queue={...queue,revision:queue.revision+1,inputs:edit.inputIds.flatMap((id:string)=>queue.inputs.filter(i=>i.inputId===id))};
    else if(edit.type==="pause"||edit.type==="resume")queue={...queue,revision:queue.revision+1,paused:edit.type==="pause"};
    return structuredClone(queue);
  });
  f.queuePrepare.mockImplementation(async request=>{
    const p=await f.prepare.getMockImplementation()!(request.submission.request);
    queue={...queue,revision:queue.revision+1,preparingInputId:request.inputId};
    return {schemaVersion:1,inputId:request.inputId,scope:queue.scope,record:{conversationId,requestId:p.requestId,turnId,contextSnapshotId:p.contextSnapshotId,input:p.input},queue:structuredClone(queue)};
  });
  f.queueCancel.mockImplementation(async()=>{queue={...queue,revision:queue.revision+1,paused:true,preparingInputId:null};return structuredClone(queue);});
  return ()=>queue;
}

it("keeps a recovered queue claim in the composer and preserves the draft while retrying its original recovery",async()=>{
  await render();await click("打开 AI 笔记助手");await click("选择测试对话");await type("保留草稿");
  const input=container.querySelector("textarea")!, inputId="e1111111-1111-4111-8111-111111111111";
  f.queueRead.mockImplementation(async scope=>({...scope,schemaVersion:1,revision:1,paused:false,preparingInputId:inputId,inputs:[{inputId,text:"队列消息"}]}));
  f.queueRecover.mockRejectedValue(new Error("recovery unavailable"));
  await click("选择测试对话");
  await vi.waitFor(()=>expect(container.querySelector('[aria-label="输入框准备状态"]')?.textContent).toBe("消息发送状态待确认"));
  const composer=container.querySelector(".note-agent-session form")!;
  expect(composer.querySelector('[aria-label="输入框准备状态"]')).not.toBeNull();
  expect(container.querySelector('[aria-label="队列消息"]')?.textContent).not.toContain("消息发送状态待确认");
  expect(input.disabled).toBe(true);expect(input.value).toBe("保留草稿");
  expect(button("将消息加入队列").disabled).toBe(true);expect(button("准备发送消息").disabled).toBe(true);
  await act(async()=>container.querySelector(".note-agent-session form")!.dispatchEvent(new Event("submit",{bubbles:true,cancelable:true})));
  expect(f.queueEdit).not.toHaveBeenCalled();expect(f.prepare).not.toHaveBeenCalled();
  expect(button("处理消息").disabled).toBe(false);
  await click("处理消息");await vi.waitFor(()=>expect(f.queueRecover).toHaveBeenCalledOnce());
  expect(container.querySelector("textarea")).toBe(input);expect(input.value).toBe("保留草稿");
  await vi.waitFor(()=>expect(button("处理消息").disabled).toBe(false));
  await click("处理消息");await vi.waitFor(()=>expect(f.queueRecover).toHaveBeenCalledTimes(2));
});

it("shows queue sending in the composer while the original prepare is pending",async()=>{
  queueFixture();
  f.queuePrepare.mockImplementationOnce(()=>new Promise(()=>undefined));
  await render("saving");await click("打开 AI 笔记助手");await click("选择测试对话");
  await type("排队发送");await click("将消息加入队列");await type("下一条草稿");
  const input=container.querySelector("textarea")!;
  await render("saved");await vi.waitFor(()=>expect(f.queuePrepare).toHaveBeenCalledOnce());
  await vi.waitFor(()=>expect(container.querySelector('[aria-label="输入框准备状态"]')?.textContent).toBe("发送中…"));
  expect(input.disabled).toBe(true);expect(input.value).toBe("下一条草稿");
  expect(container.querySelector('[aria-label="队列提交预览"]')).toBeNull();
  expect(container.querySelector('[aria-label="队列消息"]')?.textContent).not.toContain("发送中…");
});

it("queues ordinary input but disables steering during manual compaction",async()=>{
  const compact=vi.fn(()=>new Promise<commandPort.AgentCompactResult>(()=>undefined));
  const port=vi.spyOn(commandPort,"getAgentCommandGateway").mockReturnValue({readUsage:async()=>({inputTokens:82000,contextWindowTokens:128000}),compact});
  queueFixture();
  try{
    await render();await click("打开 AI 笔记助手");await click("选择测试对话");
    await type("/compact");await click("准备发送消息");
    expect(container.textContent).toContain("正在压缩上下文");
    await type("压缩期间补充");
    const input=container.querySelector('textarea')!;
    await act(async()=>input.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",ctrlKey:true,bubbles:true,cancelable:true})));
    expect(input.value).toBe("压缩期间补充");
    await click("准备发送消息");
    expect(container.querySelector("textarea")?.value).toBe("");
    expect([...container.querySelectorAll('.agent-input-queue-body li p')].map(row=>row.textContent)).toEqual(["压缩期间补充"]);
    await click("将第 1 条消息插入当前轮次");
    expect(container.querySelector<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"]')?.disabled).toBe(true);
    expect(container.textContent).not.toContain("压缩完成后发送");
    expect(f.steer).not.toHaveBeenCalled();expect(f.prepare).not.toHaveBeenCalled();expect(f.queuePrepare).not.toHaveBeenCalled();
    expect(compact).toHaveBeenCalledOnce();expect(container.textContent).toContain("正在压缩上下文");
  }finally{port.mockRestore();}
});
it("queues explicitly during a running turn without sending or overwriting later draft text",async()=>{
  queueFixture();f.projection={activeTurn:turnId,turnOrder:[],turns:new Map()} as never;
  let finish!:()=>void;const edit=f.queueEdit.getMockImplementation()!;
  f.queueEdit.mockImplementationOnce(request=>new Promise(done=>{finish=()=>{void edit(request).then(done);};}));
  await render();await click("打开 AI 笔记助手");await click("选择测试对话");await type("Next turn");
  const input=container.querySelector("textarea")!;
  await click("将消息加入队列");
  await type("Later draft");await act(async()=>finish());
  expect(f.queueEdit.mock.calls[0]?.[0]).toMatchObject({conversationId,scope:{kind:"note",resourceId:pageId},expectedRevision:0,edit:{type:"add",text:"Next turn"}});
  expect(input.value).toBe("Later draft");expect(f.prepare).not.toHaveBeenCalled();expect(f.start).not.toHaveBeenCalled();expect(f.queueStart).not.toHaveBeenCalled();
  await click("隐藏 AI 笔记助手");await click("打开 AI 笔记助手");
  expect(container.querySelector("textarea")).toBe(input);expect(container.textContent).toContain("Next turn");
  expect(container.querySelector("form .note-agent-queue")).not.toBeNull();
});
it("steers a queued note item into the active turn and removes it only after committed history",async()=>{
  queueFixture();f.projection=history(null);f.steer.mockResolvedValue({});
  await render();await click("打开 AI 笔记助手");await click("选择测试对话");await type("Clarify this");await click("将消息加入队列");
  await act(async()=>{await Promise.resolve();await Promise.resolve();});
  await click("引导");
  expect(f.steer).toHaveBeenCalledWith(expect.objectContaining({conversationId,scope:{kind:"note",resourceId:pageId},turnId,text:"Clarify this"}));
  expect(f.queueEdit.mock.calls.at(-1)?.[0]).toMatchObject({edit:{type:"add"}});
  const request=f.steer.mock.calls.at(-1)?.[0] as {inputId:string;text:string};
  const base=history(null),turn=base.turns.get(turnId)!;
  f.status="syncing";
  f.projection={...base,seq:base.seq+3,turns:new Map([[turnId,{...turn,itemOrder:[...turn.itemOrder,request.inputId],items:new Map([...turn.items,[request.inputId,{kind:{type:"userMessage" as const},text:request.text,completed:true,interrupted:false}]])}]])};
  await render();await act(async()=>{await Promise.resolve();await Promise.resolve();});
  expect(f.queueEdit.mock.calls.at(-1)?.[0]).toMatchObject({edit:{type:"add"}});
  const read=f.queueRead.getMockImplementation()!;let finishRead!:()=>void;
  f.queueRead.mockImplementationOnce(scope=>new Promise(done=>{finishRead=()=>{void read(scope).then(done);};}));
  f.status="ready";await render();
  expect(finishRead).toBeTypeOf("function");expect(f.queueEdit.mock.calls.at(-1)?.[0]).toMatchObject({edit:{type:"add"}});
  await act(async()=>finishRead());
  await vi.waitFor(()=>expect(f.queueEdit.mock.calls.at(-1)?.[0]).toMatchObject({edit:{type:"remove"}}));
});
it("retains a confirmed steer when queue removal fails and reconciles without resending",async()=>{
  queueFixture();f.projection=history(null);f.steer.mockResolvedValue({});
  await render();await click("打开 AI 笔记助手");await click("选择测试对话");await type("Keep this transfer");await click("将消息加入队列");
  f.queueEdit.mockRejectedValueOnce({code:"conflict"});
  await click("引导");
  const request=f.steer.mock.calls.at(-1)?.[0] as {inputId:string;text:string};
  const base=history(null),turn=base.turns.get(turnId)!;
  f.projection={...base,seq:base.seq+3,turns:new Map([[turnId,{...turn,itemOrder:[...turn.itemOrder,request.inputId],items:new Map([...turn.items,[request.inputId,{kind:{type:"userMessage" as const},text:request.text,completed:true,interrupted:false}]])}]])};
  await render();
  await vi.waitFor(()=>expect(container.textContent).toContain("消息已进入对话，但暂时无法确认是否已从队列移除。"));
  expect(f.steer).toHaveBeenCalledOnce();
  expect([...container.querySelectorAll("button")].filter(item=>item.textContent==="引导").every(item=>item.disabled)).toBe(true);
  await click("核对队列移除");
  await vi.waitFor(()=>expect(container.textContent).not.toContain("消息已进入对话，但暂时无法确认是否已从队列移除。"));
  expect(f.queueEdit.mock.calls.at(-1)?.[0]).toMatchObject({edit:{type:"remove"}});
  expect(f.steer).toHaveBeenCalledOnce();
});
it("retries the original uncertain queue removal after a confirmed steer",async()=>{
  queueFixture();f.projection=history(null);f.steer.mockResolvedValue({});
  await render();await click("打开 AI 笔记助手");await click("选择测试对话");await type("Keep original identity");await click("将消息加入队列");
  const edit=f.queueEdit.getMockImplementation()!;let attempts=0;
  f.queueEdit.mockImplementation(async request=>{
    if(request.edit.type==="remove"&&attempts++<3){if(attempts===1)await edit(request);throw new Error("lost reply");}
    return edit(request);
  });
  await click("引导");
  const request=f.steer.mock.calls.at(-1)?.[0] as {inputId:string;text:string};
  const base=history(null),turn=base.turns.get(turnId)!;
  f.projection={...base,seq:base.seq+3,turns:new Map([[turnId,{...turn,itemOrder:[...turn.itemOrder,request.inputId],items:new Map([...turn.items,[request.inputId,{kind:{type:"userMessage" as const},text:request.text,completed:true,interrupted:false}]])}]])};
  await render();
  await vi.waitFor(()=>expect(container.textContent).toContain("消息已进入对话，但暂时无法确认是否已从队列移除。"),{timeout:5000});
  const original=f.queueEdit.mock.calls.find(([value])=>value.edit.type==="remove")![0];
  await click("核对队列移除");
  await vi.waitFor(()=>expect(container.textContent).not.toContain("消息已进入对话，但暂时无法确认是否已从队列移除。"));
  const removals=f.queueEdit.mock.calls.filter(([value])=>value.edit.type==="remove");
  expect(removals.length).toBeGreaterThan(3);expect(removals.every(([value])=>JSON.stringify(value)===JSON.stringify(original))).toBe(true);
  expect(f.steer).toHaveBeenCalledOnce();
});
it("automatically dispatches a queued note once its source is saved",async()=>{
  queueFixture();await render("dirty");await click("打开 AI 笔记助手");await click("选择测试对话");await type("Queued block question");
  await click("将消息加入队列");expect(container.querySelector("textarea")?.value).toBe("");
  await render("saved",4);
  await act(async()=>{await Promise.resolve();await Promise.resolve();});
  expect(f.queuePrepare.mock.calls[0]?.[0]).toMatchObject({expectedRevision:1,submission:{kind:"note",request:{text:"Queued block question",context:{pageId,expectedRevision:4,blockId:null}}}});
  expect(f.queueStart).toHaveBeenCalledOnce();
  expect(container.textContent).not.toContain("立即发送队首");
});
it("waits for synchronized history before consuming a queued note outcome",async()=>{
  const observe=vi.spyOn(NoteQueueSubmissionController.prototype,"observeOutcome");
  try {
    queueFixture();await render("dirty");await click("打开 AI 笔记助手");await click("选择测试对话");await type("Queued block question");
    await click("将消息加入队列");await render("saved",4);
    await vi.waitFor(()=>expect(f.queueStart).toHaveBeenCalledOnce());
    f.status="syncing";f.projection=history("completed");await render("saved",4);
    expect(observe).not.toHaveBeenCalled();
    f.status="ready";await render("saved",4);
    expect(observe).toHaveBeenCalledWith(turnId,"completed");
  } finally {observe.mockRestore();}
});
it.each([false,true])("keeps ordinary Tab in the composer without dispatching, running=%s",async running=>{
  queueFixture();if(running)f.projection={activeTurn:turnId,turnOrder:[],turns:new Map()} as never;
  await render();await click("打开 AI 笔记助手");await click("选择测试对话");
  const input=container.querySelector("textarea")!;
  const key=async(init:KeyboardEventInit)=>{
    const event=new KeyboardEvent("keydown",{key:"Tab",bubbles:true,cancelable:true,...init});
    await act(async()=>input.dispatchEvent(event));expect(event.defaultPrevented).toBe(!init.metaKey);
  };
  await key({});await type("Draft");await key({});await key({shiftKey:true});await key({isComposing:true});await key({metaKey:true});
  const modal=document.createElement("div");modal.setAttribute("aria-modal","true");document.body.append(modal);
  await key({});modal.remove();expect(f.queueEdit).not.toHaveBeenCalled();
  expect(input.value).toBe("Draft");expect(f.prepare).not.toHaveBeenCalled();expect(f.steer).not.toHaveBeenCalled();
});
it("retries an ambiguous enqueue with its original identity while retaining the draft",async()=>{
  queueFixture();const edit=f.queueEdit.getMockImplementation()!;
  f.queueEdit.mockRejectedValue(new Error("lost reply"));
  await render();await click("打开 AI 笔记助手");await click("选择测试对话");await type("Unconfirmed queued message");
  await click("将消息加入队列");expect(container.querySelector("textarea")?.value).toBe("Unconfirmed queued message");
  expect(button("准备发送消息").disabled).toBe(true);expect(button("将消息加入队列").disabled).toBe(true);
  await vi.waitFor(async()=>{await act(async()=>Promise.resolve());expect(f.queueEdit).toHaveBeenCalledTimes(3);},{timeout:2500});
  f.queueEdit.mockImplementation(edit);await click("重试队列同步");
  expect(f.queueEdit.mock.calls[0]).toEqual(f.queueEdit.mock.calls[3]);
  expect(container.textContent).toContain("Unconfirmed queued message");expect(container.querySelector("textarea")?.value).toBe("");
});
it("steers the active note turn with Enter while preserving later drafts and frozen-source semantics",async()=>{
  f.projection={conversationId,activeTurn:turnId,turnOrder:[],turns:new Map()} as never;
  let finish!:()=>void;f.steer.mockImplementation(()=>new Promise<void>(done=>{finish=done;}));
  await render("dirty",5);await click("打开 AI 笔记助手");await click("选择测试对话");await type("Clarify the original evidence");
  expect(button("补充说明").disabled).toBe(false);
  await act(async()=>container.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",bubbles:true,cancelable:true})));
  expect(f.steer.mock.calls[0]?.[0]).toMatchObject({conversationId,scope:{kind:"note",resourceId:pageId},turnId,text:"Clarify the original evidence"});
  expect(f.steer.mock.calls[0]?.[0]).not.toHaveProperty("context");expect(f.steer.mock.calls[0]?.[0]).not.toHaveProperty("model");
  await type("Later question");await click("隐藏 AI 笔记助手");await act(async()=>finish());await click("打开 AI 笔记助手");
  expect(container.querySelector("textarea")?.value).toBe("Later question");expect(f.prepare).not.toHaveBeenCalled();expect(f.queueEdit).not.toHaveBeenCalled();
});
it("holds unknown note steering through turn completion and retries only the original instruction",async()=>{
  queueFixture();f.projection={conversationId,activeTurn:turnId,turnOrder:[],turns:new Map()} as never;
  f.steer.mockRejectedValueOnce(new Error("lost reply")).mockRejectedValueOnce({code:"closed"}).mockResolvedValueOnce({});
  await render();await click("打开 AI 笔记助手");await click("选择测试对话");await type("Original steer");await click("补充说明");
  await type("New question");expect(button("将消息加入队列").disabled).toBe(true);
  f.projection={conversationId,activeTurn:null,turnOrder:[],turns:new Map()} as never;await render();
  expect(button("准备发送消息").disabled).toBe(true);
  await click("重试原补充说明");expect(button("准备发送消息").disabled).toBe(true);await click("重试原补充说明");
  expect(f.steer.mock.calls[0]).toEqual(f.steer.mock.calls[2]);expect(container.querySelector("textarea")?.value).toBe("New question");
  expect(button("准备发送消息").disabled).toBe(false);expect(f.prepare).not.toHaveBeenCalled();
});
it("does not expose terminal-turn steer recovery",async()=>{
  f.projection={conversationId,activeTurn:null,turnOrder:[turnId],turns:new Map([[turnId,{outcome:"canceled",itemOrder:[],items:new Map()}]])} as never;
  await render("dirty");await click("打开 AI 笔记助手");await click("选择测试对话");await type("Current draft");
  expect(container.textContent).not.toContain("找回未处理的补充说明");
  expect(f.steerRecovery).not.toHaveBeenCalled();expect(container.querySelector("textarea")?.value).toBe("Current draft");
});
it("does not steer for composing Enter or a modal, and closed admission preserves the draft",async()=>{
  queueFixture();f.projection={conversationId,activeTurn:turnId,turnOrder:[],turns:new Map()} as never;
  f.steer.mockRejectedValue({code:"closed"});await render();await click("打开 AI 笔记助手");await click("选择测试对话");await type("Clarify");
  const input=container.querySelector("textarea")!;
  await act(async()=>input.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",isComposing:true,bubbles:true})));expect(f.steer).not.toHaveBeenCalled();
  const modal=document.createElement("div");modal.setAttribute("aria-modal","true");document.body.append(modal);
  await act(async()=>input.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",bubbles:true})));modal.remove();expect(f.steer).not.toHaveBeenCalled();
  await click("补充说明");expect(input.value).toBe("Clarify");expect(container.textContent).toContain("已停止接收");expect(button("将消息加入队列").disabled).toBe(false);
});
