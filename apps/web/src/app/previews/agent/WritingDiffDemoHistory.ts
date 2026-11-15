import type { MessageInspectionRequest } from "../../../core/features/agent/messageInspectionGateway";
import {
  applyAgentEvents, createAgentConversationProjection, decodeAgentEvent, decodeAgentHistoryResponse, decodeAgentHistoryWaitResponse, decodeAgentSessionResponse,
  type AgentEvent, type AgentEventPayload, type AgentHistoryWaitRequest, type AgentHistoryWaitResponse,
  type AgentModelIdentity, type AgentUserInput,type AgentSkillInstructions,type AgentSteeringHandoffIntent,
} from "@research-assistant/core/agent";
import type { AgentSessionGateway } from "@research-assistant/data/agent";
import type { WritingSubmissionAdmission } from "../../../core/features/agent/writingSubmissionController";
import type { DemoChange } from "../../../core/features/agent/WritingDiffFixtures";
import { writingDemoPatch } from "./WritingDemoPatchContract";

interface Waiter {
  readonly request: AgentHistoryWaitRequest;
  readonly finish: (watermark: number | null) => void;
}
/** Simulated durable event I/O only. Production history controllers consume this
 * gateway; there is no preview-specific projection, message list or Agent loop. */
export class WritingDiffDemoHistory {
  snapshot() { return structuredClone({ events: this.events, admissions: this.admissions, finished: this.finished, deleted: this.deleted }); }
  restore(snapshot: ReturnType<WritingDiffDemoHistory["snapshot"]>): void {
    this.events.clear(); for (const entry of snapshot.events) this.events.set(entry[0], entry[1]);
    this.admissions.clear(); for (const entry of snapshot.admissions) this.admissions.set(entry[0], entry[1]);
    this.finished.clear(); for (const entry of snapshot.finished) this.finished.add(entry);
    this.deleted.clear(); for (const entry of snapshot.deleted) this.deleted.add(entry);
  }

  onChange: (() => void) | undefined;
  private readonly events = new Map<string, readonly AgentEvent[]>();
  private readonly admissions = new Map<string, WritingSubmissionAdmission>();
  private readonly finished = new Set<string>();
  private readonly deleted = new Set<string>();
  private readonly waiters = new Set<Waiter>();
  private disposed = false;

  interruptUnfinished(): void {
    for (const admission of this.admissions.values()) if (this.activeTurn(admission.scope.conversationId) === admission.turnId) this.finish(admission, "canceled");
  }
  inspectMessage(request: MessageInspectionRequest): readonly AgentEvent[] {
    this.assertAvailable(request.conversationId);
    const events = (this.events.get(request.conversationId) ?? []).filter(event =>
      event.payload.turnId === request.turnId && "itemId" in event.payload && event.payload.itemId === request.itemId);
    if (!events.some(event => event.payload.type === "itemStarted")) throw "notFound";
    return structuredClone(events);
  }
  isFirstUserMessage(request: MessageInspectionRequest): boolean {
    this.inspectMessage(request);
    const first = (this.events.get(request.conversationId) ?? []).find(event => event.payload.turnId === request.turnId
      && event.payload.type === "itemStarted" && event.payload.kind.type === "userMessage");
    return first?.payload.type === "itemStarted" && first.payload.itemId === request.itemId;
  }
  private turnKey(value: WritingSubmissionAdmission): string { return JSON.stringify([value.scope.conversationId, value.turnId]); }
  private watermark(id: string): number { return this.events.get(id)?.length ?? 0; }
  assertAvailable(id: string): void { if (this.deleted.has(id)) throw "notFound"; }
  activeTurn(id: string): string | null {
    this.assertAvailable(id);
    return applyAgentEvents(createAgentConversationProjection(id), this.events.get(id) ?? []).activeTurn;
  }
  turnIsTerminal(id:string,turnId:string):boolean{
    this.assertAvailable(id);
    const turn=applyAgentEvents(createAgentConversationProjection(id),this.events.get(id)??[]).turns.get(turnId);
    return !!turn && turn.outcome!==null;
  }
  /** Mirrors the native per-item consumption check without inferring from text
   * alone; the caller separately verifies the accepted input and its resources. */
  steeringHandoffHistory(intent:AgentSteeringHandoffIntent):"confirmed"|"unconfirmed"|"incompleteOrConflicting"{
    this.assertAvailable(intent.conversationId);
    const events=(this.events.get(intent.conversationId)??[]).filter(event=>"itemId" in event.payload && event.payload.itemId===intent.steeringInputId);
    let started=false,complete=false,text="";
    for(const event of events){
      const payload=event.payload;
      if(payload.turnId!==intent.turnId||complete)return "incompleteOrConflicting";
      if(payload.type==="itemStarted"&&payload.kind.type==="userMessage"&&!started){started=true;continue;}
      if(payload.type==="textDelta"&&started){text+=payload.text;if(!intent.text.startsWith(text))return "incompleteOrConflicting";continue;}
      if(payload.type==="itemCompleted"&&started&&text===intent.text){complete=true;continue;}
      return "incompleteOrConflicting";
    }
    return complete?"confirmed":started?"incompleteOrConflicting":"unconfirmed";
  }
  isDeleted(id: string): boolean { return this.deleted.has(id); }
  fork(parentId: string, childId: string, throughTurnId: string): number {
    this.assertAvailable(parentId); this.assertAvailable(childId);
    if (parentId === childId || this.watermark(childId)) throw "conflict";
    const events = this.events.get(parentId) ?? [];
    const end = events.findIndex(event => event.payload.type === "turnFinished" && event.payload.turnId === throughTurnId);
    if (end < 0) throw "busy";
    const copied = events.slice(0, end + 1).map(event => decodeAgentEvent({ ...event, conversationId: childId }));
    applyAgentEvents(createAgentConversationProjection(childId), copied);
    this.events.set(childId, structuredClone(copied));
    return end + 1;
  }
  assertIdle(id: string): void {
    this.assertAvailable(id);
    const projection = applyAgentEvents(createAgentConversationProjection(id), this.events.get(id) ?? []);
    if ([...projection.turns.values()].some(turn => turn.outcome === null)) throw "busy";
  }
  assertWatermark(id: string, expected: number): void {
    this.assertAvailable(id);
    if (this.watermark(id) !== expected) throw "conflict";
  }
  deleteConversation(id: string, expectedWatermark: number): void {
    this.assertAvailable(id);
    if (this.watermark(id) !== expectedWatermark) throw "conflict";
    this.assertIdle(id);
    this.deleted.add(id); this.events.delete(id);
    for (const [key, value] of this.admissions) {
      if (value.scope.conversationId === id) { this.admissions.delete(key); this.finished.delete(key); }
    }
    for (const waiter of [...this.waiters]) if (waiter.request.conversationId === id) waiter.finish(null);
  }
  private append(value: WritingSubmissionAdmission, payloads: readonly AgentEventPayload[]): void {
    if (this.disposed) throw new Error("模拟历史存储已关闭");
    const conversationId = value.scope.conversationId;
    this.assertAvailable(conversationId);
    const previous = this.events.get(conversationId) ?? [];
    const timestampMs = Date.now();
    // Validate the entire append before publishing any event or waking readers.
    const batch = payloads.map((payload, index) => decodeAgentEvent({ schemaVersion: 1, conversationId,
      seq: previous.length + index + 1, timestampMs, requestId: value.scope.requestId, payload }));
    applyAgentEvents(createAgentConversationProjection(conversationId), [...previous, ...batch]);
    this.events.set(conversationId, [...previous, ...batch]);
    this.onChange?.();
    for (const waiter of [...this.waiters]) {
      if (waiter.request.conversationId === conversationId && this.watermark(conversationId) > waiter.request.afterSeq) waiter.finish(this.watermark(conversationId));
    }
  }
  private message(turnId: string, text: string, kind: "userMessage" | "assistantMessage", phase?: "commentary" | "finalAnswer"): AgentEventPayload[] {
    const itemId = crypto.randomUUID();
    return [{ type: "itemStarted", turnId, itemId, kind: { type: kind } },
      ...(phase ? [{ type: "itemPhaseChanged" as const, turnId, itemId, phase }] : []),
      ...this.textDeltas(turnId, itemId, text), { type: "itemCompleted", turnId, itemId }];
  }
  private textDeltas(turnId: string, itemId: string, text: string): AgentEventPayload[] {
    const chars = Array.from(text), output: AgentEventPayload[] = [];
    for (let start = 0; start < chars.length; start += 65_536) output.push({type: "textDelta", turnId, itemId, text: chars.slice(start, start + 65_536).join("")});
    return output;
  }
  start(value: WritingSubmissionAdmission, text: string, model: AgentModelIdentity = { providerId: "fixture", modelId: "writing-diff-demo", revision: "1" }): void {
    const key = this.turnKey(value), existing = this.admissions.get(key);
    if (existing) {
      if (JSON.stringify(existing) !== JSON.stringify(value)) throw new Error("模拟历史轮次身份冲突");
      return;
    }
    this.append(value, [{ type: "turnStarted", turnId: value.turnId, contextSnapshotId: value.scope.contextSnapshotId, model },
      ...this.message(value.turnId, text, "userMessage"), {type:"phaseChanged",turnId:value.turnId,phase:"runningModel"}]);
    this.admissions.set(key, structuredClone(value));
  }
  /** Synthetic native Compact facts; command text never becomes a user item. */
  startCompaction(value: WritingSubmissionAdmission, model: AgentModelIdentity, itemId: string): void {
    if (this.admissions.has(this.turnKey(value))) return;
    this.assertIdle(value.scope.conversationId);
    this.append(value, [{ type: "turnStarted", turnId: value.turnId, contextSnapshotId: value.scope.contextSnapshotId, model, taskKind: "compact" },
      { type: "phaseChanged", turnId: value.turnId, phase: "runningModel" },
      { type: "itemStarted", turnId: value.turnId, itemId, kind: { type: "contextCompaction" } }]);
    this.admissions.set(this.turnKey(value), structuredClone(value));
  }
  finishCompaction(value: WritingSubmissionAdmission, itemId: string, outcome: "completed" | "failed" | "canceled" | "interrupted"): void {
    const key = this.turnKey(value);
    if (this.disposed || this.finished.has(key)) return;
    this.append(value, [...(outcome === "completed" ? [{ type: "itemCompleted" as const, turnId: value.turnId, itemId }] : []),
      { type: "turnFinished", turnId: value.turnId, outcome }]);
    this.finished.add(key);
  }
  steer(value:WritingSubmissionAdmission,itemId:string,text:string):void{if(this.finished.has(this.turnKey(value)))throw "closed";this.append(value,[{type:"itemStarted",turnId:value.turnId,itemId,kind:{type:"userMessage"}},{type:"textDelta",turnId:value.turnId,itemId,text},{type:"itemCompleted",turnId:value.turnId,itemId}]);}
  toolFailure(value:WritingSubmissionAdmission):void{const callId=crypto.randomUUID(),itemId=crypto.randomUUID(),resultId=crypto.randomUUID();this.append(value,[{type:"itemStarted",turnId:value.turnId,itemId,kind:{type:"toolCall",callId,name:"mock_script"}},{type:"textDelta",turnId:value.turnId,itemId,text:"{}"},{type:"itemCompleted",turnId:value.turnId,itemId},{type:"itemStarted",turnId:value.turnId,itemId:resultId,kind:{type:"toolResult",callId}},{type:"textDelta",turnId:value.turnId,itemId:resultId,text:JSON.stringify({code:"tool_unavailable",message:"模拟脚本依赖缺失"})},{type:"itemCompleted",turnId:value.turnId,itemId:resultId},...this.message(value.turnId,"Agent 请求失败：模拟脚本依赖缺失。","assistantMessage")]);}
  private toolRound(value: WritingSubmissionAdmission, name: string, arguments_: unknown, output: unknown, isError?: boolean): void {
    const callId = crypto.randomUUID(), itemId = crypto.randomUUID(), resultId = crypto.randomUUID();
    this.append(value, [
      {type: "phaseChanged", turnId: value.turnId, phase: "runningTools"},
      {type: "itemStarted", turnId: value.turnId, itemId, kind: {type: "toolCall", callId, name}},
      ...this.textDeltas(value.turnId, itemId, JSON.stringify(arguments_)),
      {type: "itemCompleted", turnId: value.turnId, itemId},
      {type: "itemStarted", turnId: value.turnId, itemId: resultId, kind: {type: "toolResult", callId, ...(isError ? { isError: true } : {})}},
      ...this.textDeltas(value.turnId, resultId, JSON.stringify(output)),
      {type: "itemCompleted", turnId: value.turnId, itemId: resultId},
      {type: "phaseChanged", turnId: value.turnId, phase: "runningModel"},
    ]);
  }
  responseStep(value: WritingSubmissionAdmission, step: {
    readonly name: string; readonly arguments: unknown; readonly output: unknown; readonly text: string;
    readonly phase?: "commentary" | "finalAnswer";
    readonly isError?: boolean;
  }): void {
    if (this.finished.has(this.turnKey(value))) throw "closed";
    this.toolRound(value, step.name, step.arguments, step.output, step.isError);
    this.append(value, this.message(value.turnId, step.text, "assistantMessage", step.phase));
  }
  toolStep(value: WritingSubmissionAdmission, step: {
    readonly name: string; readonly arguments: unknown; readonly output: unknown; readonly isError?: boolean;
  }): void {
    if (this.finished.has(this.turnKey(value))) throw "closed";
    this.toolRound(value, step.name, step.arguments, step.output, step.isError);
  }
  feedbackEvents(value: WritingSubmissionAdmission, events: readonly AgentEventPayload[]): void {
    if (this.finished.has(this.turnKey(value))) throw "closed";
    this.append(value, events);
  }
  resourceRound(value: WritingSubmissionAdmission, input: readonly AgentUserInput[], prompts: readonly AgentSkillInstructions[]): void {
    if (prompts.length) {
      for (const prompt of prompts) {
        this.toolRound(value, "research_load_skill", {identity: prompt.identity},
          {skill: {identity: prompt.identity, name: prompt.name, body: prompt.contents}});
      }
      this.toolRound(value, "research_exec_command", {command: "sh ../skills/mock/scripts/run.sh", yield_ms: 5000}, {state: "finished", exitCode: 0, output: "mock skill completed"});
      this.append(value, this.message(value.turnId, "工具结果已接收，继续处理。", "assistantMessage"));
    }
    for (const reference of input) {
      if (reference.type === "mention" && reference.target.kind === "tool") {
        this.toolRound(value, "mock_search", {query: "paper"}, {papers: []});
        this.append(value, this.message(value.turnId, "工具结果已接收，继续处理。", "assistantMessage"));
      }
    }
  }
  complete(value: WritingSubmissionAdmission, changes: readonly DemoChange[], reported = false): void {
    this.start(value, "帮我完善摘要部分");
    const key = this.turnKey(value);
    if (this.finished.has(key)) return;
    if (reported) {
      this.append(value, [{ type: "turnFinished", turnId: value.turnId, outcome: "completed" }]);
      this.finished.add(key);
      return;
    }
    const callId = crypto.randomUUID(), callItem = crypto.randomUUID(), resultItem = crypto.randomUUID();
    const patch = writingDemoPatch(value.scope.manuscriptId, changes);
    this.append(value, [
      { type: "phaseChanged", turnId: value.turnId, phase: "runningTools" },
      { type: "itemStarted", turnId: value.turnId, itemId: callItem, kind: { type: "toolCall", callId, name: patch.name } },
      { type: "textDelta", turnId: value.turnId, itemId: callItem, text: JSON.stringify(patch.arguments) },
      { type: "itemCompleted", turnId: value.turnId, itemId: callItem },
      { type: "itemStarted", turnId: value.turnId, itemId: resultItem, kind: { type: "toolResult", callId } },
      { type: "textDelta", turnId: value.turnId, itemId: resultItem, text: JSON.stringify(patch.output) },
      { type: "itemCompleted", turnId: value.turnId, itemId: resultItem },
      { type: "phaseChanged", turnId: value.turnId, phase: "runningModel" },
      ...this.message(value.turnId, `已准备 ${changes.length} 个文件的修改。`, "assistantMessage"),
      { type: "turnFinished", turnId: value.turnId, outcome: "completed" },
    ]);
    this.finished.add(key);
  }
  finish(value: WritingSubmissionAdmission, outcome: "failed" | "canceled"): void {
    const key = this.turnKey(value);
    if (this.disposed || this.finished.has(key)) return;
    this.append(value, [{ type: "turnFinished", turnId: value.turnId, outcome }]);
    this.finished.add(key);
  }
  private wait(request: AgentHistoryWaitRequest, signal?: AbortSignal): Promise<AgentHistoryWaitResponse> {
    if (signal?.aborted) return Promise.reject(signal.reason ?? new DOMException("Aborted", "AbortError"));
    this.assertAvailable(request.conversationId);
    const watermark = this.watermark(request.conversationId);
    if (this.disposed) return Promise.reject(new Error("模拟历史存储已关闭"));
    if (watermark > request.afterSeq) return Promise.resolve(decodeAgentHistoryWaitResponse({ schemaVersion: 1,
      conversationId: request.conversationId, afterSeq: request.afterSeq, watermark }));
    if (!Number.isSafeInteger(request.waitMs) || request.waitMs < 0 || request.waitMs > 30_000) return Promise.reject(new Error("模拟历史等待范围无效"));
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => waiter.finish(null), request.waitMs);
      const cleanup = (): void => { clearTimeout(timer); this.waiters.delete(waiter); signal?.removeEventListener("abort", abort); };
      const abort = (): void => { cleanup(); reject(signal?.reason ?? new DOMException("Aborted", "AbortError")); };
      const waiter: Waiter = { request: { ...request }, finish: (next) => {
        cleanup();
        resolve(decodeAgentHistoryWaitResponse({ schemaVersion: 1, conversationId: request.conversationId, afterSeq: request.afterSeq, watermark: next }));
      } };
      this.waiters.add(waiter);
      signal?.addEventListener("abort", abort, { once: true });
    });
  }
  readonly gateway: AgentSessionGateway = {
    create: async (conversationId) => { this.assertAvailable(conversationId); return decodeAgentSessionResponse({ schemaVersion: 1, conversationId, watermark: this.watermark(conversationId) }); },
    open: async (conversationId) => { this.assertAvailable(conversationId); return decodeAgentSessionResponse({ schemaVersion: 1, conversationId, watermark: this.watermark(conversationId) }); },
    readHistory: async (request) => {
      this.assertAvailable(request.conversationId);
      const all = this.events.get(request.conversationId) ?? [], events = all.slice(request.afterSeq, request.afterSeq + request.limit);
      return structuredClone(decodeAgentHistoryResponse({ schemaVersion: 1, conversationId: request.conversationId,
        afterSeq: request.afterSeq, watermark: all.length, events,
        nextAfterSeq: request.afterSeq + events.length < all.length ? request.afterSeq + events.length : null }));
    },
    waitForHistory: (request, signal) => this.wait(request, signal),
  };
  /** Resetting the simulated backend releases remaining pending I/O. */
  dispose(): void {
    this.disposed = true;
    for (const waiter of [...this.waiters]) waiter.finish(null);
  }
}
