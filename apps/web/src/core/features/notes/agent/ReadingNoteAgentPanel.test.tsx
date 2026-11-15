/** @vitest-environment happy-dom */
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { noteBlockId, type ReadingNoteBlock } from "@research-assistant/core/notes";
import type { AgentReadingNotePrepareRequest, AgentConversationProjection, AgentSearchHit } from "@research-assistant/core/agent";
import { applyAgentEvents, createAgentConversationProjection, decodeAgentEvent } from "@research-assistant/core/agent";
import { ReadingNoteAgentPanel } from "./NoteAgentPanel";
import { AgentDraftProvider } from "../../agent/AgentDraftProvider";
import { resetReadingNoteSubmissionOwnersForTests } from "../../agent/readingNoteSubmissionController";
import { resetNoteQueueSubmissionOwnersForTests } from "../../agent/noteQueueSubmissionController";
import { agentCreationGateway, agentMetadataGateway } from "../../../services/agentGateway";
import { creationProof } from "../../agent/conversationCreationTestFacts";
import { getAppStorage } from "../../../runtime/appStorage";
import { resetConversationCreationOwnersForTests } from "../../agent/conversationCreationController";

const f = vi.hoisted(()=>({inspectChild:vi.fn(),steer:vi.fn(),steerRecovery:vi.fn(),queueRead:vi.fn(),queueEdit:vi.fn(),queuePrepare:vi.fn(),queueRecover:vi.fn(),queueCancel:vi.fn(),queueStart:vi.fn(),queueRestore:vi.fn(),queueList:vi.fn(),prepare:vi.fn(),restore:vi.fn(),start:vi.fn(),interrupt:vi.fn(),refresh:vi.fn(),recovery:vi.fn(),review:vi.fn(),childList:vi.fn(),resumeReadingNote:vi.fn(),projection:null as AgentConversationProjection|null,hit:undefined as AgentSearchHit|undefined}));
const conversationId = "a1111111-1111-4111-8111-111111111111";
const readingNoteId = noteBlockId("b1111111-1111-4111-8111-111111111111");
const turnId = "c1111111-1111-4111-8111-111111111111";
vi.mock("../../../services/agentGateway",async importOriginal=>({...(await importOriginal<typeof import("../../../services/agentGateway")>()),agentChildCreationStatusGateway:{read:f.inspectChild},agentNoteSteeringGateway:{steer:f.steer,recover:f.steerRecovery},agentNoteQueueGateway:{read:f.queueRead,edit:f.queueEdit,prepare:f.queuePrepare,recover:f.queueRecover,cancel:f.queueCancel,start:f.queueStart,restore:f.queueRestore,list:f.queueList},agentReadingNoteSubmissionGateway:{prepare:f.prepare,restore:f.restore,start:f.start},agentSubtaskGateway:{list:f.childList,resumeReadingNote:f.resumeReadingNote},agentTurnGateway:{interrupt:f.interrupt},agentReadingNoteRecoveryGateway:{list:f.recovery},agentNoteReviewGateway:{review:f.review}}));
vi.mock("../../agent/BoundConversationList",()=>({BoundConversationList:({onSelect}:{onSelect:(id:string,archived?:boolean,hit?:AgentSearchHit,pending?:boolean)=>void})=><><button onClick={()=>onSelect("a1111111-1111-4111-8111-111111111111",false,f.hit)}>选择测试对话</button><button onClick={()=>onSelect("a1111111-1111-4111-8111-111111111111",false,undefined,true)}>选择本地新对话</button></>}));
vi.mock("../../agent/useAgentConversation",()=>({useAgentConversation:()=>({status:"ready",projection:f.projection})}));
vi.mock("../../agent/useConversationMetadata",()=>({useConversationMetadata:()=>({status:"ready",conversation:{archived:false},refresh:f.refresh})}));
vi.mock("../../agent/AgentModelControl",()=>({AgentModelControl:()=> <span>模型选择</span>}));
vi.mock("../../speech/PaperAiReadingSpeechInput",()=>({PaperAiReadingSpeechInput:()=> <span>语音输入</span>}));
vi.mock("../../../services/notifications",()=>({notify:{error:vi.fn(),success:vi.fn()}}));
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?:boolean}).IS_REACT_ACT_ENVIRONMENT=true;

let container:HTMLDivElement;
let root:ReturnType<typeof createRoot>;
function note(revision=3,id=readingNoteId):ReadingNoteBlock {
  return {id,noteType:"annotation",body:"Saved interpretation",color:"#ffcc00",status:"active",isFavorite:false,revision,projectIds:[],tagIds:[],createdAt:"now",updatedAt:"now",
    source:{paperId:"paper",paperTitle:"Synthetic reading",attachmentId:"pdf",contentSha256:"a".repeat(64),pageIndex:2,rects:[],quoteExact:"Retained quotation",quotePrefix:"",quoteSuffix:"",state:"unavailable"}};
}
async function render(isSaved=true,revision=3,id=readingNoteId,autoDispatch=false) {
  await act(async()=>root.render(<StrictMode><AgentDraftProvider><ReadingNoteAgentPanel note={note(revision,id)} isSaved={isSaved} autoDispatch={autoDispatch}/></AgentDraftProvider></StrictMode>));
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
  resetReadingNoteSubmissionOwnersForTests(); resetNoteQueueSubmissionOwnersForTests();
  vi.clearAllMocks();f.steer.mockReset();f.steerRecovery.mockReset();f.projection=null;f.hit=undefined;
  f.queueRead.mockImplementation(async scope=>({...scope,schemaVersion:1,revision:0,paused:false,preparingInputId:null,inputs:[]}));
  f.queueList.mockImplementation(async scope=>({schemaVersion:1,conversationId:scope.conversationId,scope:scope.scope,items:[],nextCursor:null}));
  f.childList.mockResolvedValue([]);
  container=document.createElement("div");document.body.append(container);root=createRoot(container);
  f.prepare.mockImplementation(async(request:AgentReadingNotePrepareRequest)=>({schemaVersion:1,conversationId,readingNoteId,requestId:request.requestId,turnId,contextSnapshotId:"d1111111-1111-4111-8111-111111111111",input:{text:request.text,model:{providerId:"deepseek",modelId:"deepseek-v4-pro",revision:null},reasoningEffort:null,context:[{kind:"note",sourceId:`reading-note:${readingNoteId}`,resourceId:readingNoteId,revision:`revision:${request.context.expectedRevision}`,text:"Frozen interpretation and retained quote",anchor:null}]}}));
  f.start.mockImplementation(async preview=>({schemaVersion:1,conversationId,requestId:preview.requestId,turnId,alreadyStarted:false,outcome:null}));
  f.interrupt.mockResolvedValue({schemaVersion:1,conversationId,turnId,cancellationRequested:true});
});
afterEach(async()=>{await act(async()=>root.unmount());container.remove();});

it("shows the unconfirmed tool receipt after reopening a stopped turn",async()=>{
  const callId="e1111111-1111-4111-8111-111111111111", itemId="f1111111-1111-4111-8111-111111111111";
  const payloads=[
    {type:"turnStarted",turnId,contextSnapshotId:itemId,model:{providerId:"test",modelId:"test",revision:null}},
    {type:"itemAccepted",turnId,itemId,kind:{type:"toolCall",callId,name:"read"},text:"{}",phase:null,source:null},
    {type:"toolReceiptPending",turnId,callId},
    {type:"turnFinished",turnId,outcome:"interrupted"},
  ];
  f.projection=applyAgentEvents(createAgentConversationProjection(conversationId),payloads.map((payload,index)=>decodeAgentEvent({schemaVersion:1,conversationId,seq:index+1,timestampMs:1,requestId:turnId,payload})));
  await render();await click("打开 AI 阅读笔记助手");await click("选择测试对话");
  expect(container.textContent).toContain("回执尚未确认");expect(container.textContent).toContain(callId);
  await type("保留后续草稿");expect(button("准备发送消息").disabled).toBe(true);
  await act(async()=>container.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",bubbles:true})));
  expect(container.querySelector("textarea")?.value).toBe("保留后续草稿");
  expect(f.prepare).not.toHaveBeenCalled();expect(f.start).not.toHaveBeenCalled();
});

it.each(["creation", "cleanup", "unrelated"] as const)("inspects a saved %s reading-note child while preserving distinct failures",async scenario=>{
  const callId="e1111111-1111-4111-8111-111111111111", itemId="f1111111-1111-4111-8111-111111111111";
  const childId="d1111111-1111-4111-8111-111111111111", operationId="99999999-9999-4999-8999-999999999999";
  const submissionRequestId="a2222222-2222-4222-8222-222222222222", archiveId="a3333333-3333-5333-8333-333333333333";
  const detail = scenario === "cleanup" ? {code:"DATA_SQL_COMMIT_UNKNOWN",requestId:submissionRequestId,resultId:childId,
    childArchiveIntentVersion:1,childArchiveOperationId:archiveId,journalRequestId:archiveId,childArchiveExpectedWatermark:0}
    : {code:"CHILD_CREATION_UNCONFIRMED",conversationId,turnId,requestId:scenario === "creation"?operationId:callId,resultId:childId};
  const payloads=[
    {type:"turnStarted",turnId,contextSnapshotId:itemId,model:{providerId:"test",modelId:"test",revision:null}},
    {type:"itemAccepted",turnId,itemId,kind:{type:"toolCall",callId,name:"research_spawn_subtask",providerCallId:"original"},text:"{}",phase:null,source:null},
    {type:"childCreationPrepared",turnId,callId,version:2,childId,operationId,submissionRequestId,binding:{kind:"readingNote",resourceId:readingNoteId}},
    {type:"turnFailed",turnId,category:"recovery",error:`tool_outcome_unknown: ${JSON.stringify(detail)}`},
    {type:"turnFinished",turnId,outcome:"failed"},
  ];
  f.projection=applyAgentEvents(createAgentConversationProjection(conversationId),payloads.map((payload,index)=>decodeAgentEvent({schemaVersion:1,conversationId,seq:index+1,timestampMs:1,requestId:turnId,payload})));
  f.inspectChild.mockResolvedValue({schemaVersion:1,conversationId,turnId,callId,watermark:payloads.length,
    creation:{schemaVersion:1,conversationId:childId,operationId,status:{type:"noSqlReceipt",epoch:itemId,sqlFence:"0",closed:false}}});
  await render();await click("打开 AI 阅读笔记助手");await click("选择测试对话");await type("保留草稿");
  expect(container.textContent?.includes("Agent 请求失败")).toBe(scenario === "unrelated");
  if (scenario !== "unrelated") {
    const writeText=vi.fn(async(_value:string)=>undefined);Object.defineProperty(navigator,"clipboard",{configurable:true,value:{writeText}});
    await act(async()=>container.querySelector<HTMLButtonElement>(`[aria-label="子任务${scenario === "cleanup" ? "清理" : "创建"}恢复"] [aria-label="复制诊断信息"]`)!.click());
    expect(writeText.mock.lastCall?.[0]).toContain(detail.code);
  }
  if (scenario === "cleanup") expect(container.textContent).toContain("子任务清理结果尚待核对");
  await click(scenario === "cleanup" ? "核对子任务状态" : "核对子任务创建");
  expect(f.inspectChild).toHaveBeenCalledExactlyOnceWith({schemaVersion:1,conversationId,turnId,callId});
  expect(container.textContent).toContain("未找到原创建回执");expect(button("准备发送消息").disabled).toBe(true);
  expect(container.querySelector("textarea")?.value).toBe("保留草稿");expect(f.start).not.toHaveBeenCalled();
});

it("recovers reading-note creation using its exact binding without sending or changing its pending C",async()=>{
  const create=vi.spyOn(agentCreationGateway,"createBound").mockRejectedValueOnce(new Error("lost reply"));
  const status=vi.spyOn(agentCreationGateway,"createBoundStatus").mockImplementation(async intent=>({schemaVersion:1,operationId:intent.operationId,conversationId:intent.id,status:{type:"committed",proof:creationProof(intent)}}));
  const read=vi.spyOn(agentMetadataGateway,"read").mockImplementation(async request=>({id:request.id,title:"Current reading note",archived:false,durableCount:0,createdAt:"now",updatedAt:"now"}));
  try {
    await render();await click("打开 AI 阅读笔记助手");await click("选择本地新对话");await type("Retained reading question");await click("准备发送消息");
    const intent=create.mock.calls[0]![0];expect(intent.binding).toEqual({kind:"readingNote",resourceId:readingNoteId});
    await click("准备发送消息");expect(create).toHaveBeenCalledTimes(1);await click("核对原创建操作");
    expect(status).toHaveBeenCalledExactlyOnceWith(intent);expect(read).not.toHaveBeenCalled();await click("读取当前对话");
    expect(read).toHaveBeenCalledExactlyOnceWith({id:conversationId,binding:intent.binding});
    expect(f.prepare).not.toHaveBeenCalled();expect(f.start).not.toHaveBeenCalled();expect(container.querySelector("textarea")?.value).toBe("Retained reading question");
    await click("准备发送消息");expect(f.prepare).toHaveBeenCalledOnce();expect(create).toHaveBeenCalledTimes(1);
  } finally {create.mockRestore();status.mockRestore();read.mockRestore();}
});

it("preserves the reading draft and mounted input while hidden, with model and speech controls",async()=>{
  await render();await click("打开 AI 阅读笔记助手");await click("选择测试对话");await type("Reading draft");
  const input=container.querySelector("textarea");
  expect(container.textContent).toContain("模型选择");expect(container.textContent).toContain("语音输入");
  expect(container.textContent).toContain("当前阅读笔记");expect(container.querySelector('select[aria-label="笔记来源范围"]')).toBeNull();
  await click("隐藏 AI 阅读笔记助手");expect(document.activeElement).toBe(button("打开 AI 阅读笔记助手"));
  await click("打开 AI 阅读笔记助手");expect(container.querySelector("textarea")).toBe(input);expect(input?.value).toBe("Reading draft");
  expect(f.prepare).not.toHaveBeenCalled();expect(f.interrupt).not.toHaveBeenCalled();
});
it("waits for saved reading content then confirms the original revision while retaining new drafts",async()=>{
  await render(false);await click("打开 AI 阅读笔记助手");await click("选择测试对话");await type("Explain retained evidence");
  expect(button("准备发送消息").disabled).toBe(true);
  await render(true,4);await click("准备发送消息");
  expect(f.prepare).toHaveBeenCalledWith(expect.objectContaining({conversationId,text:"Explain retained evidence",context:{readingNoteId,expectedRevision:4}}));
  expect(f.start).not.toHaveBeenCalled();expect(container.textContent).toContain("Frozen interpretation and retained quote");
  await type("Next question");await render(false,5);await click("确认发送");
  expect(f.start.mock.calls[0]?.[0].input.context[0].revision).toBe("revision:4");
  expect(container.querySelector("textarea")?.value).toBe("Next question");
  await click("停止生成");await click("停止生成");expect(f.interrupt).toHaveBeenCalledExactlyOnceWith({conversationId,turnId});
});
it("recovers an old reading receipt without sending or replacing the current question",async()=>{
  const requestId="e1111111-1111-4111-8111-111111111111";
  f.recovery.mockResolvedValue({schemaVersion:1,conversationId,readingNoteId,nextCursor:null,items:[{requestId,turnId,messageExcerpt:"Old reading question",createdAt:"2026-09-11T12:00:00Z"}]});
  f.restore.mockResolvedValue({schemaVersion:1,conversationId,readingNoteId,requestId,turnId,contextSnapshotId:"old",input:{text:"Old reading question",model:{providerId:"deepseek",modelId:"deepseek-v4-pro",revision:null},reasoningEffort:null,context:[{text:"Original retained evidence",revision:"revision:1"}]}});
  await render(false);await click("打开 AI 阅读笔记助手");await click("选择测试对话");await type("Current draft");
  await click("检查未启动的提交");expect(f.recovery).toHaveBeenCalledExactlyOnceWith({conversationId,readingNoteId,after:null,limit:20});
  await click("查看原提交");expect(f.restore).toHaveBeenCalledExactlyOnceWith({conversationId,readingNoteId,requestId});
  expect(f.prepare).not.toHaveBeenCalled();expect(f.start).not.toHaveBeenCalled();
  await click("确认发送");expect(container.querySelector("textarea")?.value).toBe("Current draft");
});
it("keeps an uncertain start on its original reading receipt and ignores composing Enter",async()=>{
  await render();await click("打开 AI 阅读笔记助手");await click("选择测试对话");await type("Explain");
  await act(async()=>container.querySelector("textarea")!.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",isComposing:true,bubbles:true})));
  expect(f.prepare).not.toHaveBeenCalled();await click("准备发送消息");f.start.mockRejectedValueOnce(new Error("lost reply"));
  await click("确认发送");expect(container.textContent).not.toContain("返回输入");
  await click("查询或继续原轮次");expect(f.start.mock.calls[0]).toEqual(f.start.mock.calls[1]);expect(f.prepare).toHaveBeenCalledOnce();
});
it("isolates source switches and preserves each draft without sending a late preparation",async()=>{
  let resolve!:(value:unknown)=>void;
  const prepared=f.prepare.getMockImplementation()!;
  f.prepare.mockImplementationOnce(()=>new Promise(done=>{resolve=done;}));
  await render();await click("打开 AI 阅读笔记助手");await click("选择测试对话");await type("First draft");await click("准备发送消息");
  const request=f.prepare.mock.calls[0]![0];
  await render(true,3,noteBlockId("f1111111-1111-4111-8111-111111111111"));
  await click("打开 AI 阅读笔记助手");await click("选择测试对话");expect(container.querySelector("textarea")?.value).toBe("");await type("Second draft");
  await act(async()=>resolve(await prepared(request)));
  expect(container.querySelector("textarea")?.value).toBe("Second draft");
  expect(Array.from(container.querySelectorAll("button")).some(el=>el.textContent==="确认发送")).toBe(false);
  expect(f.start).not.toHaveBeenCalled();expect(f.interrupt).not.toHaveBeenCalled();
  await render();await click("打开 AI 阅读笔记助手");await click("选择测试对话");expect(container.querySelector("textarea")?.value).toBe("First draft");
});

it("automatically prepares and starts a reading queue using frozen retained evidence",async()=>{
  const inputId="f1111111-1111-4111-8111-111111111111";
  const queue={schemaVersion:1,conversationId,scope:{kind:"readingNote",resourceId:readingNoteId},revision:1,paused:false,preparingInputId:null,inputs:[{inputId,text:"Queued reading question"}]};
  f.queueRead.mockResolvedValue(queue);
  f.queuePrepare.mockImplementation(async request=>{
    const p=await f.prepare.getMockImplementation()!(request.submission.request);
    return {schemaVersion:1,inputId,scope:queue.scope,record:{conversationId,requestId:p.requestId,turnId,contextSnapshotId:p.contextSnapshotId,input:p.input},queue:{...queue,revision:2,preparingInputId:inputId}};
  });
  f.queueStart.mockImplementation(async(_scope,record)=>({schemaVersion:1,conversationId,requestId:record.requestId,turnId,alreadyStarted:false,outcome:null}));
  await render(true,4);await click("打开 AI 阅读笔记助手");await click("选择测试对话");
  await act(async()=>{await Promise.resolve();await Promise.resolve();});
  expect(f.queuePrepare.mock.calls[0]?.[0]).toMatchObject({inputId,expectedRevision:1,submission:{kind:"readingNote",request:{text:"Queued reading question",context:{readingNoteId,expectedRevision:4}}}});
  await type("Later reading draft");
  expect(f.queueStart.mock.calls[0]?.[0]).toEqual({conversationId,scope:queue.scope});
  expect(f.queueStart.mock.calls[0]?.[1].input.context[0].revision).toBe("revision:4");
  expect(f.queueStart.mock.calls[0]?.[2]).toEqual({inputId,expectedRevision:2});
  expect(f.prepare).not.toHaveBeenCalled();expect(f.start).not.toHaveBeenCalled();expect(container.querySelector("textarea")?.value).toBe("Later reading draft");
});
it("recovers an admitted reading queue from its original receipt and leaves the current draft intact",async()=>{
  const requestId="e1111111-1111-4111-8111-111111111111",inputId="f1111111-1111-4111-8111-111111111111";
  const admission={requestId,turnId,createdAt:"2026-09-11T12:00:00Z",messageExcerpt:"Queued original",queue:{inputId,expectedRevision:7}};
  f.queueList.mockResolvedValue({schemaVersion:1,conversationId,scope:{kind:"readingNote",resourceId:readingNoteId},items:[admission],nextCursor:null});
  f.queueRestore.mockResolvedValue({conversationId,requestId,turnId,contextSnapshotId:inputId,input:{text:"Queued original",model:{providerId:"deepseek",modelId:"deepseek-v4-pro",revision:null},reasoningEffort:null,context:[{text:"Original retained quote",revision:"revision:1"}]}});
  await render(false);await click("打开 AI 阅读笔记助手");await click("选择测试对话");await type("Current reading draft");
  await click("检查队列中未启动的提交");await click("查看队列原提交");
  expect(f.queueRestore).toHaveBeenCalledExactlyOnceWith({conversationId,scope:{kind:"readingNote",resourceId:readingNoteId}},requestId);
  expect(container.textContent).toContain("Original retained quote");expect(f.queuePrepare).not.toHaveBeenCalled();
  await click("返回队列");expect(container.querySelector("textarea")?.value).toBe("Current reading draft");
});
it("steers a reading-note turn without materializing its newer unsaved annotation",async()=>{
  f.projection={conversationId,activeTurn:turnId,turnOrder:[],turns:new Map()} as never;
  f.steer.mockResolvedValue({});await render(false,9);await click("打开 AI 阅读笔记助手");await click("选择测试对话");await type("Explain retained quote only");
  await click("补充说明");expect(f.steer.mock.calls[0]?.[0]).toMatchObject({conversationId,scope:{kind:"readingNote",resourceId:readingNoteId},turnId,text:"Explain retained quote only"});
  expect(container.querySelector("textarea")?.value).toBe("");expect(f.prepare).not.toHaveBeenCalled();expect(f.start).not.toHaveBeenCalled();expect(f.queuePrepare).not.toHaveBeenCalled();
});
it("does not expose terminal-turn steer recovery for reading notes",async()=>{
  f.projection={conversationId,activeTurn:null,turnOrder:[turnId],turns:new Map([[turnId,{outcome:"canceled",itemOrder:[],items:new Map()}]])} as never;
  f.steerRecovery.mockResolvedValue({inputs:[{inputId:"retained",text:"Pending reading instruction"}]});
  await render();await click("打开 AI 阅读笔记助手");await click("选择测试对话");await type("New reading question");
  expect(container.textContent).not.toContain("找回未处理的补充说明");
  expect(f.steerRecovery).not.toHaveBeenCalled();
  expect(container.querySelector("textarea")?.value).toBe("New reading question");expect(f.steer).not.toHaveBeenCalled();expect(f.start).not.toHaveBeenCalled();
});

it("explicitly resumes a reading-note child with the frozen reading-note binding",async()=>{
  const childConversationId="d1111111-1111-4111-8111-111111111111";
  f.childList.mockResolvedValue([{schemaVersion:1,childConversationId,parentConversationId:conversationId,parentContextSnapshotId:turnId,childContextSnapshotId:childConversationId,depth:1,modelCalls:1,toolCalls:1,status:"recovery_required",createdAt:"2026-09-13T00:00:00Z",finishedAt:"2026-09-13T00:01:00Z"}]);
  f.resumeReadingNote.mockResolvedValue(undefined);
  await render();await click("打开 AI 阅读笔记助手");await click("选择测试对话");
  await act(async()=>Promise.resolve());
  await click("恢复并继续");
  expect(f.resumeReadingNote).toHaveBeenCalledExactlyOnceWith(conversationId,childConversationId,readingNoteId);
});
it("drops a late child recovery result after switching reading notes",async()=>{
  const childConversationId="d1111111-1111-4111-8111-111111111111";
  f.childList.mockResolvedValue([{schemaVersion:1,childConversationId,parentConversationId:conversationId,parentContextSnapshotId:turnId,childContextSnapshotId:childConversationId,depth:1,modelCalls:1,toolCalls:1,status:"recovery_required",createdAt:"2026-09-13T00:00:00Z",finishedAt:"2026-09-13T00:01:00Z"}]);
  let resolve!:()=>void;
  f.resumeReadingNote.mockImplementationOnce(()=>new Promise<void>(done=>{resolve=done;}));
  await render();await click("打开 AI 阅读笔记助手");await click("选择测试对话");
  await act(async()=>Promise.resolve());
  const before=f.childList.mock.calls.length;
  await click("恢复并继续");
  await render(true,3,noteBlockId("c1111111-1111-4111-8111-111111111111"));
  await act(async()=>{resolve();await Promise.resolve();});
  expect(f.childList.mock.calls.length).toBe(before);
});
