/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { LiteratureItem } from "@research-assistant/core/literature";
import { decodeAgentEvent } from "@research-assistant/core/agent";
import { AgentSteeringController, type AgentHistoryFeedOptions } from "@research-assistant/data/agent";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import rawEvents from "../../../../../fixtures/agent-events-v1.json";
import { creationProof } from "../../../../../../apps/web/src/core/features/agent/conversationCreationTestFacts";
import { getAppStorage } from "../../../../../../apps/web/src/core/runtime/appStorage";
import { resetConversationCreationOwnersForTests } from "../../../../../../apps/web/src/core/features/agent/conversationCreationController";
import { archiveProof } from "../../../../../../apps/web/src/core/features/agent/conversationArchiveTestFacts";
import { resetConversationArchiveOwnersForTests } from "../../../../../../apps/web/src/core/features/agent/conversationArchiveController";
import {registerPaperQueueSendTest} from "./paperQueueSendTest";
import { notify, type NotificationOptions } from "@research-assistant/workbench-sdk/feedback";
import * as appRuntime from "../../../../../../apps/web/src/core/runtime/appRuntime";
import {registerStoragePersistence} from "../../../../../../apps/web/src/core/runtime/storagePersistence";
import {NATIVE_PREVIEW_OWNER_CAPABILITY, readPreviewOwnerIntent} from "../../../../../../apps/web/src/core/runtime/nativePreviewOwnerIntent";

const gateway = vi.hoisted(() => ({
  prepareSubmission: vi.fn(),
  start: vi.fn(),
  createBound: vi.fn(),
  createBoundStatus: vi.fn(),
  metadata: vi.fn(),
  list: vi.fn(),
  setArchived: vi.fn(),
  archiveStatus: vi.fn(),
  sources: vi.fn(),
  childList: vi.fn(),
  childSubscribe: vi.fn(),
  history: vi.fn(),
  inspectChild: vi.fn(),
  prepareStatus: vi.fn(),
  startStatus: vi.fn(),
}));

vi.mock("../../../../../../apps/web/src/core/services/agentGateway", () => ({
  agentChildCreationStatusGateway: { read: gateway.inspectChild },
  agentArchiveGateway: { setArchived: gateway.setArchived, archiveStatus: gateway.archiveStatus },
  agentCatalogGateway: { list: gateway.list },
  agentCreationGateway: { createBound: gateway.createBound, createBoundStatus: gateway.createBoundStatus },
  agentMetadataGateway: { read: gateway.metadata },
  agentPaperContextGateway: {
    prepare: vi.fn(async () => ({ snapshotId: `paper-context-${"a".repeat(64)}`, contextHash: "a".repeat(64) })),
    preview: vi.fn(async () => ({ items: [], attachments: [] })),
  },
  agentPaperReadingHistoryMigrationGateway: { migrate: vi.fn(async () => undefined) },
  agentPaperQueueGateway: {
    read: vi.fn(async (scope) => ({ schemaVersion: 1, ...scope, revision: 0, paused: false, preparingInputId: null, inputs: [] })),
    edit: vi.fn(async (request) => ({ schemaVersion: 1, conversationId: request.conversationId, paperId: request.paperId, revision: request.expectedRevision + 1, paused: false, preparingInputId: null, inputs: [] })), inspectEdit: vi.fn(),
  },
  agentPaperRecoveryGateway: { list: vi.fn(async () => ({ items: [] })) },
  agentPaperSubmissionGateway: {
    prepare: gateway.prepareSubmission,
    start: gateway.start, steer: vi.fn(), sources: gateway.sources, restore: vi.fn(),
  },
  agentPaperPrepareStatusGateway: {read: gateway.prepareStatus},
  agentPaperPrepareProvenGateway: {prepare: async (request: import("@research-assistant/core/agent").AgentPaperSubmissionPrepareRequest) => ({type: "committed", result: await gateway.prepareSubmission(request), proof: {schemaVersion: 1, operationId: request.requestId, conversationId: request.conversationId, family: "agent.paper.submission.prepare", epoch: "f1111111-1111-4111-8111-111111111111", sqlFence: "1", replayed: false, result: null}})},
  agentPaperStartStatusGateway: {read: gateway.startStatus},
  agentSessionGateway: {},
  agentTaskObservationGateway: {},
  agentSubtaskGateway: { list: gateway.childList, subscribe: gateway.childSubscribe },
  agentTurnGateway: {},
  agentMcpGateway: { list: vi.fn(async () => ({ servers: [] })) },
  agentSkillGateway: { list: vi.fn(async () => ({ revision: "r1", skills: [] })) },
}));

vi.mock("@research-assistant/data/agent", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@research-assistant/data/agent")>()),
  startAgentHistoryFeed: gateway.history,
}));
vi.mock("../../../../../../../plugins/ra-literature-library/src/services/literatureGateway", () => ({ getLiteratureGateway: () => ({ readAttachment: vi.fn() }) }));
vi.mock("../../../../../../apps/web/src/core/services/notifications", () => ({ notify: { success: vi.fn(), error: vi.fn(), info: vi.fn(), warning: vi.fn(), dismiss: vi.fn() } }));

import { PaperAgentReadingPanel, resetPaperConversationOwnersForTests } from "./PaperAgentReadingPanel";
import * as commandPort from "../../../../../../apps/web/src/core/features/agent/agentCommandGateway";
import { agentPaperQueueGateway } from "../../../../../../apps/web/src/core/services/agentGateway";

function paper(): LiteratureItem {
  return {
    id: "paper-1", itemType: "journalArticle", title: "Paper", abstractText: "", containerTitle: "",
    volume: "", issue: "", pages: "", doi: "", url: "", language: "", bibtexKey: "", creators: [], tags: [],
    readingStatus: "unread", projectIds: [], metadataSources: [],
    attachments: [{ id: "attachment-1", displayName: "paper.pdf", mimeType: "application/pdf", byteSize: 10, contentSha256: "b".repeat(64) }],
    createdAt: "2026-09-16T00:00:00.000Z", updatedAt: "2026-09-16T00:00:00.000Z",
  };
}

async function flush(): Promise<void> {
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

describe("PaperAgentReadingPanel retrieval migration", () => {
  let host: HTMLDivElement;
  let root: Root;
  const selectionStorage = new Map<string, string>();
  const localStorage = {
    get length() { return selectionStorage.size; },
    key: (index: number) => [...selectionStorage.keys()][index] ?? null,
    getItem: (key: string) => selectionStorage.get(key) ?? null,
    setItem: (key: string, value: string) => { selectionStorage.set(key, value); },
    removeItem: (key: string) => { selectionStorage.delete(key); },
    clear: () => { selectionStorage.clear(); },
  } as Storage;

  beforeEach(() => {
    resetPaperConversationOwnersForTests();
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    host = document.createElement("div"); document.body.append(host); root = createRoot(host);
    Object.defineProperty(window, "localStorage", { configurable: true, value: localStorage });
    resetConversationCreationOwnersForTests(getAppStorage()); resetConversationArchiveOwnersForTests(getAppStorage()); getAppStorage().clear();
    vi.mocked(agentPaperQueueGateway.read).mockImplementation(async scope => ({schemaVersion: 1, ...scope, revision: 0, paused: false, preparingInputId: null, inputs: []}));
    vi.mocked(agentPaperQueueGateway.edit).mockImplementation(async request => ({schemaVersion: 1, conversationId: request.conversationId, paperId: request.paperId,
      revision: request.expectedRevision + 1, paused: false, preparingInputId: null, inputs: []}));
    gateway.createBound.mockImplementation(async request => creationProof(request));
    gateway.createBoundStatus.mockImplementation(async request => ({ schemaVersion: 1, operationId: request.operationId, conversationId: request.id,
      status: { type: "committed", proof: creationProof(request) } }));
    gateway.metadata.mockImplementation(async request => ({ id: request.id, title: "Paper", archived: false, durableCount: 0,
      createdAt: "2026-09-16T00:00:00Z", updatedAt: "2026-09-16T00:00:00Z" }));
    gateway.list.mockResolvedValue({ conversations: [] });
    gateway.setArchived.mockImplementation(async request => archiveProof(request));
    gateway.archiveStatus.mockImplementation(async request => ({schemaVersion: 1, operationId: request.operationId, conversationId: request.id,
      status: {type: "committed", proof: archiveProof(request)}}));
    gateway.sources.mockResolvedValue({});
    gateway.childList.mockResolvedValue([]);
    gateway.childSubscribe.mockResolvedValue(() => {});
    gateway.history.mockImplementation((_gateway, options: AgentHistoryFeedOptions) => {
      queueMicrotask(() => options.onSyncState?.(true));
      return {afterSeq: 0, stop: vi.fn(), done: new Promise<number>(() => undefined)};
    });
    gateway.prepareSubmission.mockImplementation(async request => ({
      schemaVersion: 1, conversationId: request.conversationId,
      requestId: request.requestId, paperId: request.paperId,
      paperContextSnapshotId: `paper-context-${"a".repeat(64)}`,
      turnId: "c1111111-1111-4111-8111-111111111111", contextSnapshotId: "d1111111-1111-4111-8111-111111111111",
      input: { text: request.text, userInput: request.userInput ?? [], model: { providerId: "deepseek", modelId: "deepseek-v4-pro", revision: null }, reasoningEffort: null, context: [] },
    }));
    gateway.start.mockImplementation(async preview => ({ schemaVersion: 1, conversationId: preview.conversationId, requestId: preview.requestId,
      turnId: preview.turnId, alreadyStarted: false, outcome: null }));
    gateway.prepareStatus.mockImplementation(async intent => ({schemaVersion: 1, intent, status: !gateway.prepareSubmission.mock.calls.length ? {type: "unconfirmed", detail: {operationId: intent.requestId, epoch: "f1111111-1111-4111-8111-111111111111", sqlFence: "0", phase: "paper-prepare-status", sqliteCode: null, sqliteExtendedCode: null, safeCause: "paper_prepare_worker_unconfirmed"}} : {type: "committed", result: await gateway.prepareSubmission.mock.results[0]!.value, proof: {schemaVersion: 1, operationId: intent.requestId, conversationId: intent.conversationId, family: "agent.paper.submission.prepare", epoch: "f1111111-1111-4111-8111-111111111111", sqlFence: "1", replayed: true, result: null}}}));
  });

  afterEach(async () => { await act(async () => root.unmount()); host.remove(); selectionStorage.clear(); vi.clearAllMocks(); });

  function creationHost() {
    const aiGateway = { preparePaperReadingContext: vi.fn(async () => ({ attachmentStatuses: [], contextItems: [], imageRequests: [],
      model: { providerId: "deepseek", modelId: "deepseek-v4-pro" } })), previewConceptRetrieval: vi.fn(), cancelConceptRetrievalPreview: vi.fn(),
      getLlmConfiguration: vi.fn(async () => ({ providers: [], defaultModels: [], useCases: [] })), fetchLlmProviderModels: vi.fn(async () => ({ providerId: "deepseek", models: [] })) };
    return <PaperAgentReadingPanel item={paper()} executeReaderCommand={() => false} onClose={() => {}} aiGateway={aiGateway as never} doclingGateway={{parseAttachment:vi.fn()} as never}/>;
  }
  async function sendCreationDraft() {
    await act(async () => {
      const input=host.querySelector("textarea")!;Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")!.set!.call(input,"Retained paper question");
      input.dispatchEvent(new Event("input",{bubbles:true}));
    });
    await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="发送"]')!.click());await flush();
  }
  async function creationAction(label:string) {await act(async()=>[...host.querySelectorAll<HTMLButtonElement>("button")].find(button=>button.textContent===label)!.click());await flush();}
  registerPaperQueueSendTest({gateway, render: async () => {await act(async () => root.render(creationHost()));}, flush, getHost: () => host});

  it("keeps queued steering and history confirmation disabled until paper history is synchronized", async () => {
    const observe=vi.spyOn(AgentSteeringController.prototype,"observeHistory");
    const id="a1111111-1111-4111-8111-111111111111",turnId="b1111111-1111-4111-8111-111111111111";
    gateway.list.mockResolvedValue({conversations:[{id,title:"Paper",archived:false,updatedAt:"now"}]});
    gateway.history.mockImplementation(() => ({afterSeq:0,stop:vi.fn(),done:new Promise<number>(()=>undefined)}));
    vi.mocked(agentPaperQueueGateway.read).mockResolvedValue({schemaVersion:1,conversationId:id,paperId:"paper-1",revision:1,paused:false,preparingInputId:null,
      inputs:[{inputId:"e1111111-1111-4111-8111-111111111111",text:"Queued question"}]});
    window.localStorage.setItem("research-assistant.agent-session.paper.paper-1",id);
    try {
      await act(async()=>root.render(creationHost()));await flush();
      const feed=gateway.history.mock.calls.at(-1)?.[1] as AgentHistoryFeedOptions;
      const event=decodeAgentEvent({schemaVersion:1,conversationId:id,seq:1,timestampMs:1,requestId:turnId,
        payload:{type:"turnStarted",turnId,contextSnapshotId:turnId,model:{providerId:"test",modelId:"test",revision:null}}});
      await act(async()=>{feed.onSyncState?.(false);feed.onBatch([event]);});
      const steer=host.querySelector<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"]')!;
      expect(steer.disabled).toBe(true);expect(observe).not.toHaveBeenCalled();
      await act(async()=>feed.onSyncState?.(true));
      expect(steer.disabled).toBe(false);expect(observe).toHaveBeenCalled();
    } finally {observe.mockRestore();}
  });

  it("keeps the paper draft editable and blocks sending when a stopped tool receipt needs recovery", async () => {
    const id="a1111111-1111-4111-8111-111111111111",turnId="b1111111-1111-4111-8111-111111111111";
    const callId="c1111111-1111-4111-8111-111111111111",itemId="d1111111-1111-4111-8111-111111111111";
    gateway.list.mockResolvedValue({conversations:[{id,title:"Paper",archived:false,updatedAt:"now"}]});
    gateway.history.mockImplementation(() => ({afterSeq: 0, stop: vi.fn(), done: new Promise<number>(() => undefined)}));
    vi.mocked(agentPaperQueueGateway.read).mockResolvedValue({schemaVersion:1,conversationId:id,paperId:"paper-1",revision:1,paused:false,preparingInputId:null,
      inputs:[{inputId:"e1111111-1111-4111-8111-111111111111",text:"排队的问题"}]});
    window.localStorage.setItem("research-assistant.agent-session.paper.paper-1",id);
    await act(async()=>root.render(creationHost()));await flush();
    expect(agentPaperQueueGateway.read).toHaveBeenCalled();expect(gateway.prepareSubmission).not.toHaveBeenCalled();
    const history=gateway.history.mock.calls.at(-1)?.[1] as AgentHistoryFeedOptions;
    const input=host.querySelector("textarea")!;
    await act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")!.set!.call(input,"保留论文草稿");input.dispatchEvent(new Event("input",{bubbles:true}));});
    expect(host.querySelector<HTMLButtonElement>('button[aria-label="发送"]')!.disabled).toBe(true);
    expect(agentPaperQueueGateway.read).toHaveBeenCalled();
    expect(gateway.prepareSubmission).not.toHaveBeenCalled();
    const payloads=[{type:"turnStarted",turnId,contextSnapshotId:itemId,model:{providerId:"test",modelId:"test",revision:null}},
      {type:"itemAccepted",turnId,itemId,kind:{type:"toolCall",callId,name:"read"},text:"{}",phase:null,source:null},
      {type:"toolReceiptPending",turnId,callId},{type:"turnFinished",turnId,outcome:"interrupted"}];
    await act(async()=>{history.onBatch(payloads.map((payload,index)=>decodeAgentEvent({schemaVersion:1,conversationId:id,seq:index+1,timestampMs:1,requestId:turnId,payload})));history.onSyncState?.(true);});
    expect(input.disabled).toBe(false);expect(host.textContent).toContain("工具操作尚待核对");
    expect(host.querySelector<HTMLButtonElement>('button[aria-label="发送"]')!.disabled).toBe(true);
    await act(async()=>input.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",bubbles:true})));
    expect(input.value).toBe("保留论文草稿");expect(gateway.prepareSubmission).not.toHaveBeenCalled();expect(gateway.start).not.toHaveBeenCalled();
  });

  it.each(["creation", "cleanup", "unrelated"] as const)("places %s child recovery in its paper turn and preserves distinct failures", async scenario => {
    const id="a1111111-1111-4111-8111-111111111111", turnId="b1111111-1111-4111-8111-111111111111";
    const callId="c1111111-1111-4111-8111-111111111111", itemId="d1111111-1111-4111-8111-111111111111";
    const childId="e1111111-1111-4111-8111-111111111111", operationId="f1111111-1111-4111-8111-111111111111";
    const submissionRequestId="a2222222-2222-4222-8222-222222222222", archiveId="a3333333-3333-5333-8333-333333333333";
    const detail = scenario === "cleanup" ? {code:"DATA_SQL_COMMIT_UNKNOWN",requestId:submissionRequestId,resultId:childId,
      childArchiveIntentVersion:1,childArchiveOperationId:archiveId,journalRequestId:archiveId,childArchiveExpectedWatermark:0}
      : {code:"CHILD_CREATION_UNCONFIRMED",conversationId:id,turnId,requestId:scenario === "creation"?operationId:callId,resultId:childId};
    gateway.list.mockResolvedValue({conversations:[{id,title:"Paper",archived:false,updatedAt:"now"}]});
    gateway.history.mockImplementation(() => ({afterSeq:0,stop:vi.fn(),done:new Promise<number>(()=>undefined)}));
    window.localStorage.setItem("research-assistant.agent-session.paper.paper-1",id);
    await act(async()=>root.render(creationHost()));await flush();
    const history=gateway.history.mock.calls.at(-1)?.[1] as AgentHistoryFeedOptions;
    const payloads=[{type:"turnStarted",turnId,contextSnapshotId:itemId,model:{providerId:"test",modelId:"test",revision:null}},
      {type:"itemAccepted",turnId,itemId,kind:{type:"toolCall",callId,name:"research_spawn_subtask",providerCallId:"original"},text:"{}",phase:null,source:null},
      {type:"childCreationPrepared",turnId,callId,version:2,childId,operationId,submissionRequestId,binding:{kind:"paper",resourceId:"paper-1"}},
      {type:"turnFailed",turnId,category:"recovery",error:`tool_outcome_unknown: ${JSON.stringify(detail)}`},
      {type:"turnFinished",turnId,outcome:"failed"}];
    gateway.inspectChild.mockResolvedValue({schemaVersion:1,conversationId:id,turnId,callId,watermark:payloads.length,
      creation:{schemaVersion:1,conversationId:childId,operationId,status:{type:"noSqlReceipt",epoch:itemId,sqlFence:"0",closed:false}}});
    await act(async()=>{history.onBatch(payloads.map((payload,index)=>decodeAgentEvent({schemaVersion:1,conversationId:id,seq:index+1,timestampMs:1,requestId:turnId,payload})));history.onSyncState?.(true);});
    const recovery = host.querySelector(`[aria-label="子任务${scenario === "cleanup" ? "清理" : "创建"}恢复"]`)!;
    expect(recovery.closest(".paper-ai-reading-messages > article")).not.toBeNull();
    expect(host.textContent?.includes("Agent 请求失败")).toBe(scenario === "unrelated");
    if (scenario !== "unrelated") {
      const writeText=vi.fn(async(_value:string)=>undefined);Object.defineProperty(navigator,"clipboard",{configurable:true,value:{writeText}});
      await act(async()=>recovery.querySelector<HTMLButtonElement>('[aria-label="复制诊断信息"]')!.click());
      expect(writeText.mock.lastCall?.[0]).toContain(detail.code);
    }
    if (scenario === "cleanup") expect(recovery.textContent).toContain("子任务清理结果尚待核对");
    const button=[...host.querySelectorAll("button")].find(item=>item.textContent===(scenario === "cleanup" ? "核对子任务状态" : "核对子任务创建"))!;
    await act(async()=>button.click());
    expect(gateway.inspectChild).toHaveBeenCalledExactlyOnceWith({schemaVersion:1,conversationId:id,turnId,callId});
    expect(host.textContent).toContain("未找到原创建回执");expect(gateway.start).not.toHaveBeenCalled();
    expect(host.querySelector<HTMLButtonElement>('button[aria-label="发送"]')!.disabled).toBe(true);
  });

  it("restores the original pending paper creation across remount without choosing another C or resending",async()=>{
    gateway.createBound.mockRejectedValueOnce(new Error("lost reply"));
    await act(async()=>root.render(creationHost()));await flush();await sendCreationDraft();
    const intent=gateway.createBound.mock.calls[0]![0];await sendCreationDraft();
    expect(gateway.createBound).toHaveBeenCalledTimes(1);expect(gateway.prepareSubmission).not.toHaveBeenCalled();
    expect(host.querySelector("textarea")?.value).toBe("Retained paper question");
    await act(async()=>root.unmount());resetConversationCreationOwnersForTests(getAppStorage());root=createRoot(host);
    gateway.list.mockResolvedValue({conversations:[{id:"a1111111-1111-4111-8111-111111111111",title:"Other durable conversation",archived:false}]});
    await act(async()=>root.render(creationHost()));await flush();
    const writeText=vi.fn(async (_value:string)=>undefined);Object.defineProperty(navigator,"clipboard",{configurable:true,value:{writeText}});
    expect(host.querySelector<HTMLSelectElement>("select")?.value).toBe("");
    await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="复制诊断信息"]')!.click());
    expect(writeText.mock.lastCall?.[0]).toContain(intent.id);
    expect(gateway.createBound).toHaveBeenCalledTimes(1);expect(gateway.createBoundStatus).not.toHaveBeenCalled();expect(gateway.metadata).not.toHaveBeenCalled();
    await creationAction("核对原创建操作");expect(gateway.createBoundStatus).toHaveBeenCalledExactlyOnceWith(intent);expect(gateway.metadata).not.toHaveBeenCalled();
    await creationAction("读取当前对话");expect(gateway.metadata).toHaveBeenCalledExactlyOnceWith({id:intent.id,binding:intent.binding});
    expect(host.querySelector<HTMLSelectElement>("select")?.value).toBe(intent.id);expect(gateway.prepareSubmission).not.toHaveBeenCalled();expect(gateway.start).not.toHaveBeenCalled();
  });

  it("ends a known unreadable creation explicitly and preserves the draft before a different fresh intent",async()=>{
    gateway.metadata.mockRejectedValueOnce("notFound");
    await act(async()=>root.render(creationHost()));await flush();await sendCreationDraft();const first=gateway.createBound.mock.calls[0]![0];
    expect(gateway.prepareSubmission).not.toHaveBeenCalled();expect(host.querySelector("textarea")?.value).toBe("Retained paper question");
    await creationAction("保留草稿并结束本次创建核对");expect(gateway.createBound).toHaveBeenCalledTimes(1);expect(gateway.prepareSubmission).not.toHaveBeenCalled();
    await sendCreationDraft();const second=gateway.createBound.mock.calls[1]![0];
    expect(second.id).not.toBe(first.id);expect(second.operationId).not.toBe(first.operationId);expect(second.binding).toEqual(first.binding);
  });

  it("does not adopt an explicit recovery read after the same paper starts another local draft",async()=>{
    gateway.createBound.mockRejectedValueOnce(new Error("lost reply"));await act(async()=>root.render(creationHost()));await flush();await sendCreationDraft();
    const intent=gateway.createBound.mock.calls[0]![0];await creationAction("核对原创建操作");
    let finish!:(value:unknown)=>void;gateway.metadata.mockImplementationOnce(()=>new Promise(resolve=>{finish=resolve;}));
    await creationAction("读取当前对话");await creationAction("新会话");
    await act(async()=>finish({id:intent.id,title:"Original",archived:false,durableCount:0,createdAt:"now",updatedAt:"now"}));await flush();
    expect(host.querySelector<HTMLSelectElement>("select")?.value).toBe("");expect(window.localStorage.getItem("research-assistant.agent-session.paper.paper-1")).toBeNull();
    expect(gateway.prepareSubmission).not.toHaveBeenCalled();expect(gateway.start).not.toHaveBeenCalled();expect(host.querySelector("textarea")?.value).toBe("Retained paper question");
  });

  it("clears the acknowledged draft after fresh C adoption while Unknown creation keeps it",async()=>{
    gateway.start.mockImplementationOnce(async preview=>({schemaVersion:1,conversationId:preview.conversationId,requestId:preview.requestId,
      turnId:preview.turnId,alreadyStarted:false,outcome:null}));
    await act(async()=>root.render(creationHost()));await flush();await sendCreationDraft();
    expect(gateway.prepareSubmission).toHaveBeenCalledOnce();expect(gateway.start).toHaveBeenCalledOnce();
    expect(host.querySelector("textarea")?.value).toBe("");
  });

  it("does not reread children or pending sources for text-only history updates", async () => {
    const events = rawEvents.map(decodeAgentEvent), id = events[0]!.conversationId;
    gateway.list.mockResolvedValue({conversations: [{id, title: "Paper", archived: false, updatedAt: "now"}]});
    let finishSources!: (value: {retrievalTraceId?: string}) => void;
    const sources = new Promise<{retrievalTraceId?: string}>(resolve => { finishSources = resolve; });
    gateway.sources.mockReturnValue(sources);
    window.localStorage.setItem("research-assistant.agent-session.paper.paper-1", id);
    await act(async () => root.render(<PaperAgentReadingPanel item={paper()} executeReaderCommand={() => false} onClose={() => undefined}/>));
    await flush();
    const history = gateway.history.mock.calls.at(-1)?.[1] as AgentHistoryFeedOptions;
    await act(async () => history.onBatch(events.slice(0, 18)));
    const childReads = gateway.childList.mock.calls.length;
    expect(gateway.sources).toHaveBeenCalledTimes(1);
    for (let offset = 1; offset <= 4; offset++) await act(async () => history.onBatch([
      decodeAgentEvent({...events[17]!, seq: 18 + offset, timestampMs: 18 + offset}),
    ]));
    expect(gateway.childList).toHaveBeenCalledTimes(childReads);
    expect(gateway.sources).toHaveBeenCalledTimes(1);
    const changed = gateway.childSubscribe.mock.calls.at(-1)?.[1] as () => void;
    await act(async () => changed());
    expect(gateway.childList).toHaveBeenCalledTimes(childReads + 1);
    await act(async () => history.onBatch(events.slice(18, 25).map(event =>
      decodeAgentEvent({...event, seq: event.seq + 4, timestampMs: event.timestampMs + 4}))));
    expect(gateway.childList).toHaveBeenCalledTimes(childReads + 2);
    expect(gateway.sources).toHaveBeenCalledTimes(1);
    await act(async () => { finishSources({retrievalTraceId: "frozen-trace"}); await sources; });
    expect(gateway.sources).toHaveBeenCalledTimes(1);
  });

  it("queues normal input and disables steering while manual compaction is pending",async()=>{
    const compact=vi.fn(()=>new Promise<commandPort.AgentCompactResult>(()=>undefined));
    const port=vi.spyOn(commandPort,"getAgentCommandGateway").mockReturnValue({readUsage:async()=>({inputTokens:82000,contextWindowTokens:128000}),compact});
    const id="a1111111-1111-4111-8111-111111111111";
    let queue={schemaVersion:1 as const,conversationId:id,paperId:"paper-1",revision:0,paused:false,preparingInputId:null,inputs:[] as {inputId:string;text:string}[]};
    vi.mocked(agentPaperQueueGateway.read).mockImplementation(async()=>structuredClone(queue));
    vi.mocked(agentPaperQueueGateway.edit).mockImplementation(async request=>{
      if(request.edit.type==="add")queue={...queue,revision:queue.revision+1,inputs:[...queue.inputs,{inputId:request.edit.inputId,text:request.edit.text}]};
      return structuredClone(queue);
    });
    gateway.list.mockResolvedValue({conversations:[{id,title:"Paper",archived:false,updatedAt:"now"}]});
    try{
      await act(async()=>root.render(<PaperAgentReadingPanel item={paper()} executeReaderCommand={()=>false} onClose={()=>undefined}/>));await flush();await flush();
      const input=host.querySelector("textarea")!;
      const type=async(value:string)=>act(async()=>{Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")!.set!.call(input,value);input.dispatchEvent(new Event("input",{bubbles:true}));});
      await type("/compact");await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="发送"]')!.click());
      expect(host.textContent).toContain("正在压缩上下文");expect(input.disabled).toBe(false);
      await type("压缩中的普通引导");
      await act(async()=>input.dispatchEvent(new KeyboardEvent("keydown",{key:"Enter",ctrlKey:true,bubbles:true,cancelable:true})));
      expect(input.value).toBe("压缩中的普通引导");
      expect(queue.inputs).toHaveLength(0);
      await act(async()=>host.querySelector<HTMLButtonElement>('[aria-label="发送"]')!.click());
      expect(input.value).toBe("");expect(queue.inputs.map(row=>row.text)).toEqual(["压缩中的普通引导"]);
      const steer=host.querySelector<HTMLButtonElement>('[aria-label="将第 1 条消息插入当前轮次"]')!;
      expect(steer.disabled).toBe(true);
      await act(async()=>steer.click());
      expect(queue.inputs).toHaveLength(1);expect(gateway.prepareSubmission).not.toHaveBeenCalled();expect(gateway.start).not.toHaveBeenCalled();expect(compact).toHaveBeenCalledOnce();
    }finally{port.mockRestore();}
  });

  it("restores the remembered paper conversation when the catalog is reopened", async () => {
    const first = { id: "a1111111-1111-4111-8111-111111111111", title: "Older", updatedAt: "2026-09-16T00:00:00Z", archived: false };
    const remembered = { id: "b2222222-2222-4222-8222-222222222222", title: "With pending review", updatedAt: "2026-09-16T00:00:00Z", archived: false };
    gateway.list.mockResolvedValue({ conversations: [first, remembered] });
    window.localStorage.setItem("research-assistant.agent-session.paper.paper-1", remembered.id);
    await act(async () => root.render(<PaperAgentReadingPanel item={paper()} executeReaderCommand={() => false} onClose={() => undefined} />));
    await flush();
    await flush();
    expect(host.querySelector<HTMLSelectElement>("select")?.value).toBe(remembered.id);
  });

  it("shows archived paper conversations and restores them before sending", async () => {
    const archived = { id: "a1111111-1111-4111-8111-111111111111", title: "Imported history", updatedAt: "2026-09-16T00:00:00Z", archived: true };
    gateway.list.mockImplementation(async (request: { archived: boolean }) => ({ conversations: request.archived ? [archived] : [] }));
    await act(async () => root.render(<PaperAgentReadingPanel item={paper()} executeReaderCommand={() => false} onClose={() => undefined} />));
    await flush();
    await act(async () => Array.from(host.querySelectorAll("button")).find(button => button.textContent === "已归档")!.click());
    await flush();
    expect(host.querySelector<HTMLSelectElement>("select")?.value).toBe(archived.id);
    expect(host.querySelector('button[aria-label="恢复当前对话"]')).not.toBeNull();
    expect(host.querySelector<HTMLTextAreaElement>('textarea[aria-label="向共享 AI 提问当前文献"]')?.disabled).toBe(true);
    await act(async () => Array.from(host.querySelectorAll("button")).find(button => button.getAttribute("aria-label") === "恢复当前对话")!.click());
    expect(gateway.setArchived).toHaveBeenCalledExactlyOnceWith({ operationId: expect.any(String), id: archived.id, archived: false, expectedWatermark: null });
    expect(gateway.metadata).toHaveBeenCalledExactlyOnceWith({id: archived.id, binding: {kind: "paper", resourceId: "paper-1"}});
    expect(gateway.start).not.toHaveBeenCalled();
  });

  it("recovers the original archive from notifications after remount even when the current list lost its row", async () => {
    const row = {id: "a1111111-1111-4111-8111-111111111111", title: "Original", archived: false, durableCount: 14, createdAt: "now", updatedAt: "now"};
    gateway.list.mockResolvedValue({conversations: [row]}); gateway.setArchived.mockRejectedValueOnce(new Error("lost reply"));
    await act(async () => root.render(creationHost())); await flush();
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="归档当前对话"]')!.click()); await flush();
    const intent = gateway.setArchived.mock.calls[0]![0];
    expect(intent).toEqual({operationId: expect.any(String), id: row.id, archived: true, expectedWatermark: 14});
    expect(host.querySelector<HTMLTextAreaElement>("textarea")!.disabled).toBe(true);
    expect(gateway.metadata).not.toHaveBeenCalled();
    await act(async () => root.unmount()); resetConversationArchiveOwnersForTests(getAppStorage()); root = createRoot(host);
    gateway.list.mockResolvedValue({conversations: []}); vi.mocked(notify.error).mockClear();
    await act(async () => root.render(creationHost())); await flush();
    expect(host.querySelector<HTMLSelectElement>("select")!.value).toBe("");
    expect(notify.error).toHaveBeenCalledWith(expect.stringContaining(intent.operationId), expect.any(Object));
    const action = (label: string) => (vi.mocked(notify.error).mock.calls.at(-1)![1] as NotificationOptions).actions!.find(item => item.label === label)!;
    await act(async () => action("核对原归档操作").onClick());
    expect(gateway.archiveStatus).toHaveBeenCalledExactlyOnceWith(intent); expect(gateway.metadata).not.toHaveBeenCalled();
    gateway.metadata.mockResolvedValue({...row, title: "Later restored", archived: false});
    await act(async () => action("读取当前归档状态").onClick());
    expect(gateway.metadata).toHaveBeenCalledExactlyOnceWith({id: row.id, binding: {kind: "paper", resourceId: "paper-1"}});
    expect(host.querySelector<HTMLSelectElement>("select")!.value).toBe(""); expect(host.textContent).toContain("Later restored");
    expect(gateway.setArchived).toHaveBeenCalledTimes(1); expect(gateway.start).not.toHaveBeenCalled();
  });

  it("drops archive completion and recovery-read UI results after selection changes", async () => {
    const first = {id: "a1111111-1111-4111-8111-111111111111", title: "First", archived: false, durableCount: 14, createdAt: "now", updatedAt: "now"};
    const second = {...first, id: "b2222222-2222-4222-8222-222222222222", title: "Second"};
    gateway.list.mockResolvedValue({conversations: [first, second]}); gateway.setArchived.mockRejectedValueOnce(new Error("lost reply"));
    await act(async () => root.render(creationHost())); await flush();
    await act(async () => host.querySelector<HTMLButtonElement>('[aria-label="归档当前对话"]')!.click()); await flush();
    const action = (label: string) => (vi.mocked(notify.error).mock.calls.at(-1)![1] as NotificationOptions).actions!.find(item => item.label === label)!;
    await act(async () => action("核对原归档操作").onClick());
    let finish!: (value: unknown) => void; gateway.metadata.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    await act(async () => { void action("读取当前归档状态").onClick(); });
    await act(async () => { const select = host.querySelector<HTMLSelectElement>("select")!; select.value = second.id; select.dispatchEvent(new Event("change", {bubbles: true})); });
    vi.mocked(notify.success).mockClear(); vi.mocked(notify.error).mockClear();
    await act(async () => finish({...first, title: "Late metadata", archived: true})); await flush();
    expect(host.querySelector<HTMLSelectElement>("select")!.value).toBe(second.id); expect(host.textContent).not.toContain("Late metadata");
    expect(notify.success).not.toHaveBeenCalled(); expect(notify.error).not.toHaveBeenCalled();
    expect(gateway.setArchived).toHaveBeenCalledTimes(1); expect(gateway.start).not.toHaveBeenCalled();
  });

  it("does not restore an old conversation after the user explicitly starts a new one while loading", async () => {
    const old = { id: "a1111111-1111-4111-8111-111111111111", title: "Old conversation", updatedAt: "2026-09-16T00:00:00Z", archived: false };
    let finish!: (page: { conversations: typeof old[] }) => void;
    gateway.list.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
    window.localStorage.setItem("research-assistant.agent-session.paper.paper-1", old.id);
    await act(async () => root.render(<PaperAgentReadingPanel item={paper()} executeReaderCommand={() => false} onClose={() => undefined} />));
    await flush();
    await act(async () => Array.from(host.querySelectorAll("button")).find(button => button.textContent === "新会话")!.click());
    await act(async () => finish({ conversations: [old] }));
    expect(host.querySelector<HTMLSelectElement>("select")?.value).toBe("");
    expect(window.localStorage.getItem("research-assistant.agent-session.paper.paper-1")).toBeNull();
    expect(host.querySelector("select")?.textContent).toContain(old.title);
  });

  it("keeps a new shared Agent send available when legacy history migration fails", async () => {
    const migration = await import("../../../../../../apps/web/src/core/services/agentGateway");
    vi.mocked(migration.agentPaperReadingHistoryMigrationGateway.migrate).mockRejectedValueOnce("corrupt");
    const aiGateway = {
      preparePaperReadingContext: vi.fn(async () => ({ attachmentStatuses: [], contextItems: [], imageRequests: [], model: { providerId: "deepseek", modelId: "deepseek-v4-pro" } })),
      previewConceptRetrieval: vi.fn(), cancelConceptRetrievalPreview: vi.fn(),
      getLlmConfiguration: vi.fn(async () => ({ providers: [], defaultModels: [], useCases: [] })),
      fetchLlmProviderModels: vi.fn(async () => ({ providerId: "deepseek", models: [] })),
    };
    await act(async () => root.render(<PaperAgentReadingPanel item={paper()} executeReaderCommand={() => false} onClose={() => undefined} aiGateway={aiGateway as never} doclingGateway={{ parseAttachment: vi.fn() } as never} />));
    await flush();
    const textarea = host.querySelector<HTMLTextAreaElement>('textarea[aria-label="向共享 AI 提问当前文献"]')!;
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set?.call(textarea, "question");
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
    await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="发送"]')!.click());
    await flush();
    expect(aiGateway.preparePaperReadingContext).toHaveBeenCalledOnce();
    expect(gateway.prepareSubmission).toHaveBeenCalledOnce();
  });

  it("previews and confirms a retrieval trace before preparing the shared submission", async () => {
    const previewConceptRetrieval = vi.fn(async () => ({
      traceId: "trace-1", semanticStatus: "lexical_only", seeds: [], edges: [], sources: [],
      selectedSourceCount: 0, externalPaperCount: 0, contextSha256: "c".repeat(64), expiresAt: "2026-09-16T01:00:00Z",
    }));
    const aiGateway = {
      preparePaperReadingContext: vi.fn(async () => ({ attachmentStatuses: [], contextItems: [], imageRequests: [], model: { providerId: "deepseek", modelId: "deepseek-v4-pro" } })),
      previewConceptRetrieval,
      cancelConceptRetrievalPreview: vi.fn(),
      getLlmConfiguration: vi.fn(async () => ({
        providers: [{ providerId: "deepseek", displayName: "DeepSeek", authMode: "api-key", protocol: "openai-compatible", modelKinds: ["text", "vision"], defaultBaseUrls: ["https://invalid"], baseUrl: "https://invalid", usesDefaultBaseUrl: true, enabled: true, timeoutMs: 60_000, credentialConfigured: true }],
        defaultModels: [{ routeKind: "default", routeId: "vision", label: "Vision", modelKind: "vision", selection: { providerId: "deepseek", modelId: "deepseek-v4-pro" } }],
        useCases: [{ routeKind: "use_case", routeId: "paper_reading_chat", label: "Reading", modelKind: "vision", selection: { providerId: "deepseek", modelId: "deepseek-v4-pro" } }],
      })),
      fetchLlmProviderModels: vi.fn(async () => ({ providerId: "deepseek", models: [{ id: "deepseek-v4-pro", displayName: "DeepSeek V4 Pro", description: "", isDefault: true, capabilities: { text: true, vision: true, reasoning: false }, supportedReasoningEfforts: [] }] })),
    };
    await act(async () => root.render(<PaperAgentReadingPanel item={paper()} executeReaderCommand={() => false} onClose={() => undefined} aiGateway={aiGateway as never} doclingGateway={{ parseAttachment: vi.fn() } as never} />));
    await flush();
    const toggle = host.querySelector<HTMLInputElement>('.paper-concept-retrieval input[type="checkbox"]')!;
    const textarea = host.querySelector<HTMLTextAreaElement>('textarea[aria-label="向共享 AI 提问当前文献"]')!;
    await act(async () => {
      toggle.click();
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set?.call(textarea, "question");
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    const send = host.querySelector<HTMLButtonElement>('button[aria-label="发送"]')!;
    await act(async () => send.click()); await flush();
    expect(previewConceptRetrieval).toHaveBeenCalledWith(expect.objectContaining({ paperId: "paper-1", query: "question" }));
    const confirm = Array.from(host.querySelectorAll("button")).find((button) => button.textContent === "确认来源并发送")!;
    await act(async () => confirm.click()); await flush();
    expect(gateway.prepareSubmission).toHaveBeenCalledWith(expect.objectContaining({
      text: "question", retrieval: { traceId: "trace-1", query: "question" },
    }));
  });

  it("reserves the new conversation and its owner before retrieval and reuses both on confirmation", async () => {
    const storage = getAppStorage();
    registerStoragePersistence(storage, {backend:"IndexedDB", pending:false, error:undefined, resetting:false,
      flush:async()=>{}, retry:async()=>{}, reset:async()=>{}, subscribe:()=>()=>{}, close:()=>{}});
    let fixtureId: string | null = null;
    const preview = {bootstrap:{generation:"generation-a",supported:[NATIVE_PREVIEW_OWNER_CAPABILITY]},
      snapshotConversationOwner:()=>({generation:"generation-a",fixtureId})} as unknown as NonNullable<appRuntime.AppRuntime["nativeAgentPreview"]>;
    const original = appRuntime.getAppRuntime();
    const spy = vi.spyOn(appRuntime,"getAppRuntime").mockReturnValue({...original,storage,nativeAgentPreview:preview});
    const aiGateway = {preparePaperReadingContext:vi.fn(async()=>({attachmentStatuses:[],contextItems:[],imageRequests:[],model:{providerId:"deepseek",modelId:"deepseek-v4-pro"}})),
      previewConceptRetrieval:vi.fn(async()=>({traceId:"trace-1",semanticStatus:"lexical_only",seeds:[],edges:[],sources:[],selectedSourceCount:0,externalPaperCount:0,contextSha256:"c".repeat(64),expiresAt:"later"})),
      cancelConceptRetrievalPreview:vi.fn(),getLlmConfiguration:vi.fn(async()=>({providers:[],defaultModels:[],useCases:[]})),fetchLlmProviderModels:vi.fn(async()=>({providerId:"deepseek",models:[]}))};
    try {
      await act(async()=>root.render(<PaperAgentReadingPanel item={paper()} executeReaderCommand={()=>false} onClose={()=>undefined} aiGateway={aiGateway as never} doclingGateway={{parseAttachment:vi.fn()} as never}/>));
      await flush();
      const toggle=host.querySelector<HTMLInputElement>('.paper-concept-retrieval input[type="checkbox"]')!;
      const input=host.querySelector<HTMLTextAreaElement>('textarea[aria-label="向共享 AI 提问当前文献"]')!;
      await act(async()=>{toggle.click();Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype,"value")!.set!.call(input,"question");input.dispatchEvent(new Event("input",{bubbles:true}));});
      await act(async()=>host.querySelector<HTMLButtonElement>('button[aria-label="发送"]')!.click()); await flush();
      const key=Array.from({length:storage.length},(_,index)=>storage.key(index)).find(value=>value?.startsWith("research-assistant:preview-owner-intent:v1:"));
      expect(key).toBeDefined();
      const intent=JSON.parse(storage.getItem(key!)!) as {conversationId:string;requestId:string};
      fixtureId="33333333-3333-4333-8333-333333333333";
      expect(readPreviewOwnerIntent(storage,intent.conversationId,intent.requestId)).toEqual({fixtureId:null});
      expect(gateway.createBound).not.toHaveBeenCalled();
      const confirm=Array.from(host.querySelectorAll("button")).find(button=>button.textContent==="确认来源并发送")!;
      await act(async()=>confirm.click()); await flush();
      expect(gateway.createBound).toHaveBeenCalledWith(expect.objectContaining({id:intent.conversationId}));
      expect(gateway.prepareSubmission).toHaveBeenCalledWith(expect.objectContaining({conversationId:intent.conversationId,requestId:intent.requestId}));
    } finally {spy.mockRestore();}
  });

  it("cancels a loading retrieval preview and restores the draft", async () => {
    const cancelConceptRetrievalPreview = vi.fn(async () => undefined);
    const aiGateway = {
      preparePaperReadingContext: vi.fn(),
      previewConceptRetrieval: vi.fn(() => new Promise(() => undefined)),
      cancelConceptRetrievalPreview,
      getLlmConfiguration: vi.fn(async () => ({
        providers: [{ providerId: "deepseek", displayName: "DeepSeek", authMode: "api-key", protocol: "openai-compatible", modelKinds: ["text", "vision"], defaultBaseUrls: ["https://invalid"], baseUrl: "https://invalid", usesDefaultBaseUrl: true, enabled: true, timeoutMs: 60_000, credentialConfigured: true }],
        defaultModels: [{ routeKind: "default", routeId: "vision", label: "Vision", modelKind: "vision", selection: { providerId: "deepseek", modelId: "deepseek-v4-pro" } }],
        useCases: [{ routeKind: "use_case", routeId: "paper_reading_chat", label: "Reading", modelKind: "vision", selection: { providerId: "deepseek", modelId: "deepseek-v4-pro" } }],
      })),
      fetchLlmProviderModels: vi.fn(async () => ({ providerId: "deepseek", models: [] })),
    };
    await act(async () => root.render(<PaperAgentReadingPanel item={paper()} executeReaderCommand={() => false} onClose={() => undefined} aiGateway={aiGateway as never} doclingGateway={{ parseAttachment: vi.fn() } as never} />));
    await flush();
    const toggle = host.querySelector<HTMLInputElement>('.paper-concept-retrieval input[type="checkbox"]')!;
    const textarea = host.querySelector<HTMLTextAreaElement>('textarea[aria-label="向共享 AI 提问当前文献"]')!;
    await act(async () => {
      toggle.click();
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set?.call(textarea, "draft");
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="发送"]')!.click());
    const cancel = Array.from(host.querySelectorAll("button")).find((button) => button.textContent === "取消")!;
    await act(async () => cancel.click()); await flush();
    expect(cancelConceptRetrievalPreview).toHaveBeenCalledOnce();
    expect(textarea.value).toBe("draft");
    expect(gateway.prepareSubmission).not.toHaveBeenCalled();
  });

  it.each(["unmount", "switch", "hide"] as const)("respects pending conversation ownership on %s", async (transition) => {
    let finishCreate!: (proof: ReturnType<typeof creationProof>) => void;
    gateway.createBound.mockImplementationOnce(() => new Promise(resolve => { finishCreate = resolve; }));
    const aiGateway = {
      preparePaperReadingContext: vi.fn(async () => ({ attachmentStatuses: [], contextItems: [], imageRequests: [], model: { providerId: "deepseek", modelId: "deepseek-v4-pro" } })),
      previewConceptRetrieval: vi.fn(), cancelConceptRetrievalPreview: vi.fn(),
      getLlmConfiguration: vi.fn(async () => ({ providers: [], defaultModels: [], useCases: [] })),
      fetchLlmProviderModels: vi.fn(async () => ({ providerId: "deepseek", models: [] })),
    };
    await act(async () => root.render(<PaperAgentReadingPanel item={paper()} executeReaderCommand={() => false} onClose={() => undefined} aiGateway={aiGateway as never} doclingGateway={{ parseAttachment: vi.fn() } as never} />));
    await flush();
    const textarea = host.querySelector<HTMLTextAreaElement>('textarea[aria-label="向共享 AI 提问当前文献"]')!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set?.call(textarea, "question");
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
      host.querySelector<HTMLButtonElement>('button[aria-label="发送"]')!.click();
    });
    expect(gateway.createBound).toHaveBeenCalledOnce();
    await act(async () => root.render(transition === "unmount" ? null : <PaperAgentReadingPanel item={{ ...paper(), id: transition === "switch" ? "paper-2" : "paper-1" }} hidden={transition === "hide"} executeReaderCommand={() => false} onClose={() => undefined} aiGateway={aiGateway as never} />));
    await act(async () => finishCreate(creationProof(gateway.createBound.mock.calls[0]![0])));
    await flush();
    if (transition === "hide") {
      expect(aiGateway.preparePaperReadingContext).toHaveBeenCalledOnce();
      expect(gateway.prepareSubmission).toHaveBeenCalledOnce();
      expect(window.localStorage.getItem("research-assistant.agent-session.paper.paper-1")).not.toBeNull();
    } else {
      expect(aiGateway.preparePaperReadingContext).not.toHaveBeenCalled();
      expect(gateway.prepareSubmission).not.toHaveBeenCalled();
      expect(window.localStorage.getItem("research-assistant.agent-session.paper.paper-1")).toBeNull();
    }
    if (transition === "switch") {
      expect(host.querySelector<HTMLSelectElement>("select")?.value).toBe("");
      expect(host.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe("");
      expect(window.localStorage.getItem("research-assistant.agent-session.paper.paper-2")).toBeNull();
    }
  });

  it("keeps a prepared review attached to its conversation when switching away", async () => {
    const first = { id: "a1111111-1111-4111-8111-111111111111", title: "First", updatedAt: "2026-09-16T00:00:00Z", archived: false };
    const second = { id: "b2222222-2222-4222-8222-222222222222", title: "Second", updatedAt: "2026-09-16T00:00:00Z", archived: false };
    gateway.list.mockResolvedValue({ conversations: [first, second] });
    let resolvePrepare!: (value: Awaited<ReturnType<typeof gateway.prepareSubmission>>) => void;
    gateway.prepareSubmission.mockImplementationOnce(() => new Promise(resolve => { resolvePrepare = resolve; }));
    const aiGateway = {
      preparePaperReadingContext: vi.fn(async () => ({ attachmentStatuses: [], contextItems: [], imageRequests: [], model: { providerId: "deepseek", modelId: "deepseek-v4-pro" } })),
      previewConceptRetrieval: vi.fn(), cancelConceptRetrievalPreview: vi.fn(),
      getLlmConfiguration: vi.fn(async () => ({ providers: [], defaultModels: [], useCases: [] })),
      fetchLlmProviderModels: vi.fn(async () => ({ providerId: "deepseek", models: [] })),
    };
    await act(async () => root.render(<PaperAgentReadingPanel item={paper()} executeReaderCommand={() => false} onClose={() => undefined} aiGateway={aiGateway as never} doclingGateway={{ parseAttachment: vi.fn() } as never} />));
    await flush();
    const textarea = host.querySelector<HTMLTextAreaElement>('textarea[aria-label="向共享 AI 提问当前文献"]')!;
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set?.call(textarea, "question");
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
    await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="发送"]')!.click());
    await flush();
    const select = host.querySelector<HTMLSelectElement>("select")!;
    await act(async () => {
      select.value = second.id;
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    const requestId = gateway.prepareSubmission.mock.calls[0]![0].requestId as string;
    resolvePrepare({
      schemaVersion: 1, conversationId: first.id, requestId, paperId: "paper-1",
      paperContextSnapshotId: `paper-context-${"a".repeat(64)}`, turnId: "c1111111-1111-4111-8111-111111111111", contextSnapshotId: "d1111111-1111-4111-8111-111111111111",
      input: { text: "question", model: { providerId: "deepseek", modelId: "deepseek-v4-pro", revision: null }, reasoningEffort: null, context: [] },
    } as never);
    await flush();
    expect(select.value).toBe(second.id);
    gateway.start.mockResolvedValueOnce({ schemaVersion: 1, conversationId: first.id, requestId, turnId: "c1111111-1111-4111-8111-111111111111", alreadyStarted: false, outcome: null });
    await act(async () => {
      select.value = first.id;
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    expect(select.value).toBe(first.id);
    for (let i = 0; i < 5; i += 1) await flush();
    expect(gateway.start).toHaveBeenCalledOnce();
    expect(host.querySelector('[aria-label="论文上下文已冻结"]')).toBeNull();
  });

  it("checks a lost preparation without resending and preserves a newer draft", async () => {
    const id = "a1111111-1111-4111-8111-111111111111", turnId = "c1111111-1111-4111-8111-111111111111";
    gateway.list.mockResolvedValue({conversations: [{id, title: "Paper", archived: false, updatedAt: "now"}]});
    gateway.prepareSubmission.mockRejectedValueOnce(new Error("lost acknowledgment"));
    gateway.prepareStatus.mockImplementation(async request => ({schemaVersion: 1, intent: request, status: !gateway.prepareSubmission.mock.calls.length ? {type: "unconfirmed", detail: {operationId: request.requestId, epoch: "e1111111-1111-4111-8111-111111111111", sqlFence: "0", phase: "paper-prepare-status", sqliteCode: null, sqliteExtendedCode: null, safeCause: "paper_prepare_worker_unconfirmed"}} : {type: "committed", proof: {
      schemaVersion: 1, operationId: request.requestId, conversationId: id, family: "agent.paper.submission.prepare",
      epoch: "e1111111-1111-4111-8111-111111111111", sqlFence: "0", replayed: true, result: null,
    }, result: {schemaVersion: 1, conversationId: id, requestId: request.requestId, paperId: request.paperId,
      paperContextSnapshotId: request.paperContextSnapshotId, turnId, contextSnapshotId: "d1111111-1111-4111-8111-111111111111",
      input: {text: request.text, userInput: request.userInput ?? [], model: {providerId: "deepseek", modelId: "deepseek-v4-pro", revision: null}, reasoningEffort: null, context: []}}}}));
    await act(async () => root.render(creationHost())); await flush();
    let input = host.querySelector<HTMLTextAreaElement>("textarea")!;
    await act(async () => {Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, "Original question"); input.dispatchEvent(new Event("input", {bubbles: true}));});
    await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="发送"]')!.click());
    for (let i = 0; i < 6; i += 1) await flush();
    expect(gateway.prepareSubmission).toHaveBeenCalledOnce(); expect(gateway.start).not.toHaveBeenCalled();
    expect(host.querySelector('[aria-label="论文提交待核对"]')).not.toBeNull();
    await act(async () => root.unmount()); resetPaperConversationOwnersForTests(); root = createRoot(host);
    await act(async () => root.render(creationHost())); for (let i = 0; i < 3; i += 1) await flush();
    input = host.querySelector<HTMLTextAreaElement>("textarea")!;
    expect(host.querySelector('[aria-label="论文提交待核对"]')).not.toBeNull();
    await act(async () => {Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, "Newer draft"); input.dispatchEvent(new Event("input", {bubbles: true}));});
    await creationAction("核对原请求"); expect(gateway.prepareStatus).toHaveBeenCalledTimes(2); expect(gateway.start).not.toHaveBeenCalled();
    expect(gateway.prepareSubmission).toHaveBeenCalledOnce(); expect(gateway.start).not.toHaveBeenCalled();
    expect(input.value).toBe("Newer draft"); expect(host.querySelector('[aria-label="论文上下文已冻结"]')).not.toBeNull();
  });
  it("checks a lost start without dispatching it twice", async () => {
    const id = "a1111111-1111-4111-8111-111111111111", turnId = "c1111111-1111-4111-8111-111111111111";
    gateway.list.mockResolvedValue({conversations: [{id, title: "Paper", archived: false, updatedAt: "now"}]});
    gateway.start.mockRejectedValueOnce(new Error("lost start acknowledgment"));
    gateway.startStatus.mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "unconfirmed", detail: {
      operationId: intent.requestId, epoch: null, sqlFence: null, phase: "paper-start-status", sqliteCode: null, sqliteExtendedCode: null, safeCause: "paper_start_worker_unconfirmed"}}}));
    await act(async () => root.render(creationHost())); await flush();
    const input = host.querySelector<HTMLTextAreaElement>("textarea")!;
    await act(async () => {Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, "Question"); input.dispatchEvent(new Event("input", {bubbles: true}));});
    await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="发送"]')!.click()); for (let i = 0; i < 8; i += 1) await flush();
    expect(gateway.start).toHaveBeenCalledOnce(); await act(async () => root.unmount()); resetPaperConversationOwnersForTests(); root = createRoot(host); await act(async () => root.render(creationHost())); for (let i = 0; i < 3; i += 1) await flush();
    await creationAction("核对原请求"); expect(gateway.start).toHaveBeenCalledOnce();
    gateway.startStatus.mockImplementation(async intent => ({schemaVersion: 1, intent, status: {type: "started", result: {schemaVersion: 1, conversationId: id, requestId: intent.requestId, turnId, alreadyStarted: true, outcome: null}}}));
    await creationAction("核对原请求"); for (let i = 0; i < 6; i += 1) await flush();
    expect(gateway.start).toHaveBeenCalledOnce(); expect(host.querySelector('[aria-label="论文上下文已冻结"]')).toBeNull();
  });
});
