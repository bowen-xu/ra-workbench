import { describe, expect, it } from "vitest";
import { applyAgentEvents, createAgentConversationProjection, decodeAgentEvent, type AgentEventPayload } from "@research-assistant/core/agent";
import { projectConversationMessages } from "./projectConversationMessages";

const conversationId = "11111111-1111-4111-8111-111111111111";
const turnId = "22222222-2222-4222-8222-222222222222";
const model = { providerId: "fixture", modelId: "fixture", revision: null };
const start: AgentEventPayload = { type: "turnStarted", turnId, contextSnapshotId: "33333333-3333-4333-8333-333333333333", model };
function project(payloads: readonly AgentEventPayload[]) {
  return applyAgentEvents(createAgentConversationProjection(conversationId), [start, ...payloads].map((payload, index) => decodeAgentEvent({
    schemaVersion: 1, conversationId, requestId: "44444444-4444-4444-8444-444444444444", seq: index + 1, timestampMs: index + 1, payload,
  })));
}

describe("durable turn failures across Agent hosts", () => {
  it("offers child creation inspection only for a stopped saved intent without a completed result", () => {
    const callId = "66666666-6666-4666-8666-666666666666";
    const callItem = "55555555-5555-4555-8555-555555555555";
    const childId = "77777777-7777-4777-8777-777777777777";
    const operationId = "88888888-8888-4888-8888-888888888888";
    const resultItem = "99999999-9999-4999-8999-999999999999";
    const call: AgentEventPayload = { type: "itemAccepted", turnId, itemId: callItem,
      kind: { type: "toolCall", callId, name: "research_spawn_subtask", providerCallId: "original" }, text: "{}", phase: null, source: null };
    const intent: AgentEventPayload = { type: "childCreationPrepared", turnId, callId, version: 1, childId, operationId,
      binding: {kind: "note", resourceId: "original"} };
    const stopped: AgentEventPayload = {type: "turnFinished", turnId, outcome: "interrupted"};
    expect(projectConversationMessages(project([call, intent])).some(message => message.childCreationRecovery)).toBe(false);
    expect(projectConversationMessages(project([call, stopped])).some(message => message.childCreationRecovery)).toBe(false);
    const projection = project([call, intent, stopped]);
    expect(projectConversationMessages(projection).find(message => message.childCreationRecovery)?.childCreationRecovery)
      .toEqual([{conversationId, turnId, callId, childId, operationId, watermark: projection.seq}]);
    expect(projectConversationMessages(project([call, intent,
      {type: "itemStarted", turnId, itemId: resultItem, kind: {type: "toolResult", callId}},
      {type: "itemCompleted", turnId, itemId: resultItem}, stopped])).some(message => message.childCreationRecovery)).toBe(false);
  });
  it("merges only a terminal failure identifying the saved child creation", () => {
    const callId = "66666666-6666-4666-8666-666666666666", callItem = "55555555-5555-4555-8555-555555555555";
    const childId = "77777777-7777-4777-8777-777777777777", operationId = "88888888-8888-4888-8888-888888888888";
    const call: AgentEventPayload = {type: "itemAccepted", turnId, itemId: callItem,
      kind: {type: "toolCall", callId, name: "research_spawn_subtask", providerCallId: "original"}, text: "{}", phase: null, source: null};
    const intent: AgentEventPayload = {type: "childCreationPrepared", turnId, callId, version: 1, childId, operationId,
      binding: {kind: "note", resourceId: "original"}};
    const detail = {code: "CHILD_CREATION_UNCONFIRMED", conversationId, turnId, requestId: operationId, resultId: childId,
      phase: "creation", safeCause: "unknown"};
    const messagesFor = (reason: string) => projectConversationMessages(project([call, intent,
      {type: "turnFailed", turnId, category: "recovery", error: reason}, {type: "turnFinished", turnId, outcome: "failed"}]));
    const sameFailure = messagesFor(`tool_outcome_unknown: ${JSON.stringify(detail)}`);
    const recovery = sameFailure.find(message => message.childCreationRecovery)!;
    expect(sameFailure.filter(message => message.failure)).toHaveLength(1);
    expect(recovery.failureDiagnosticText).toContain("CHILD_CREATION_UNCONFIRMED");
    for (const unrelated of [{...detail, requestId: callId}, {...detail, resultId: callId}, {...detail, code: "OTHER"}]) {
      const messages = messagesFor(`tool_outcome_unknown: ${JSON.stringify(unrelated)}`);
      expect(messages.filter(message => message.failure)).toHaveLength(2);
      expect(messages.some(message => message.id.endsWith("/failure"))).toBe(true);
    }
    expect(messagesFor("tool_outcome_unknown: tool execution outcome requires recovery").filter(message => message.failure)).toHaveLength(2);
  });
  it("shows the original unconfirmed receipt only after a turn stops", () => {
    const callId = "66666666-6666-4666-8666-666666666666";
    const callItem = "55555555-5555-4555-8555-555555555555";
    const resultItem = "77777777-7777-4777-8777-777777777777";
    const history: AgentEventPayload[] = [
      { type: "itemAccepted", turnId, itemId: callItem, kind: { type: "toolCall", callId, name: "read" }, text: "private arguments", phase: null, source: null },
      { type: "toolReceiptPending", turnId, callId },
      { type: "itemStarted", turnId, itemId: resultItem, kind: { type: "toolResult", callId } },
      { type: "textDelta", turnId, itemId: resultItem, text: "private result" },
      { type: "itemCompleted", turnId, itemId: resultItem },
    ];
    expect(projectConversationMessages(project(history)).some(message => message.id.endsWith("receipt-recovery"))).toBe(false);
    const stopped = projectConversationMessages(project([...history, { type: "turnFinished", turnId, outcome: "interrupted" }]));
    const warning = stopped.find(message => message.id.endsWith("receipt-recovery"))!;
    expect(warning.content).toContain("回执尚未确认");
    expect(warning.content).toContain(callId);
    expect(JSON.parse(warning.failureDiagnosticText!).pendingToolReceipts).toEqual([callId]);
    expect(warning.failureDiagnosticText).not.toContain("private arguments");
    expect(warning.failureDiagnosticText).not.toContain("private result");
    const confirmed = project([...history, { type: "toolReceiptConfirmed", turnId, callId }, { type: "turnFinished", turnId, outcome: "completed" }]);
    expect(projectConversationMessages(confirmed).some(message => message.id.endsWith("receipt-recovery"))).toBe(false);
  });
  it("keeps provider continuation facts out of ordinary transcript messages", () => {
    const attemptId = "77777777-7777-4777-8777-777777777777";
    const itemId = "55555555-5555-4555-8555-555555555555";
    const state = project([
      { type: "phaseChanged", turnId, phase: "runningModel" },
      { type: "modelRequestStarted", turnId, attemptId, model },
      { type: "modelResponseStarted", turnId, attemptId, responseId: "response", model: null },
      { type: "itemAccepted", turnId, itemId, kind: { type: "providerNative", itemType: "reasoning" }, text: "", phase: null,
        source: { attemptId, ordinal: 0, providerItemId: "reasoning", providerDataJson: JSON.stringify({
          schema_version: 1, api: "responses", model, item_type: "reasoning", data: { encrypted_content: "opaque-fixture" },
        }) } },
    ]);
    expect(state.turns.get(turnId)?.items.size).toBe(1);
    expect(projectConversationMessages(state)).toEqual([]);
  });
  it("preserves distinct interrupted items from the same failed sample", () => {
    const attemptId = "77777777-7777-4777-8777-777777777777";
    const firstId = "55555555-5555-4555-8555-555555555555";
    const secondId = "66666666-6666-4666-8666-666666666666";
    const messages = projectConversationMessages(project([
      { type: "phaseChanged", turnId, phase: "runningModel" },
      { type: "modelRequestStarted", turnId, attemptId, model },
      { type: "itemStarted", turnId, itemId: firstId, kind: { type: "assistantMessage" } },
      { type: "textDelta", turnId, itemId: firstId, text: "第一条" },
      { type: "itemStarted", turnId, itemId: secondId, kind: { type: "assistantMessage" } },
      { type: "textDelta", turnId, itemId: secondId, text: "第二条" },
      { type: "modelRequestFailed", turnId, attemptId, error: "offline", retryable: true, category: null },
    ]));
    expect(messages.map(message => message.content)).toEqual(["第一条", "第二条"]);
    expect(new Set(messages.map(message => message.renderKey)).size).toBe(2);
  });
  it("copies only diagnostic metadata, without prompts, tool arguments, results or arbitrary raw fields", () => {
    const itemId = "55555555-5555-4555-8555-555555555555";
    const callId = "66666666-6666-4666-8666-666666666666";
    const projection = project([
      { type: "itemStarted", turnId, itemId, kind: { type: "toolCall", callId, name: "research_apply_patch", providerCallId: "provider-call" } },
      { type: "textDelta", turnId, itemId, text: '{"patch":"private note body"}' },
      { type: "itemCompleted", turnId, itemId },
      { type: "turnFailed", turnId, error: "tool request rejected", category: "toolRound" },
      { type: "turnFinished", turnId, outcome: "failed" },
    ]);
    const failure = projectConversationMessages(projection).at(-1)!;
    expect(failure.conversationId).toBe(conversationId);
    const data = JSON.parse(failure.failureDiagnosticText!);
    expect(data).toMatchObject({ schemaVersion: 1, conversationId, turnId,
      contextSnapshotId: start.contextSnapshotId, failure: { category: "toolRound" },
      toolCalls: [{ name: "research_apply_patch", callId, providerCallId: "provider-call" }] });
    expect(failure.failureDiagnosticText).not.toContain("private note body");
    expect(data).not.toHaveProperty("items");
    expect(data).not.toHaveProperty("messages");
  });
  it("retains exact HTTP clues, model and durable failure identifiers in the copied transcript",()=>{
    const attemptId="77777777-7777-4777-8777-777777777777";
    const messages=projectConversationMessages(project([
      {type:"phaseChanged",turnId,phase:"runningModel"},
      {type:"modelRequestStarted",turnId,attemptId,model},
      {type:"modelRequestFailed",turnId,attemptId,error:"model authentication rejected with HTTP 403",retryable:false,category:"provider"},
      {type:"turnFailed",turnId,error:"model authentication rejected with HTTP 403",category:"provider"},
      {type:"turnFinished",turnId,outcome:"failed"},
    ]));
    expect(messages).toHaveLength(1);
    expect(messages[0]?.content).toContain("HTTP 403");
    expect(messages[0]?.content).not.toContain(turnId);
    expect(messages[0]?.failureDiagnosticText).toContain(turnId);expect(messages[0]?.failureDiagnosticText).toContain(attemptId);
    expect(messages[0]?.failureDiagnosticText).toContain("model_authentication_rejected_with_http_403");
    expect(messages[0]?.failureDiagnosticText).toContain("fixture");
  });
  it("replaces only failed streamed drafts on retry and keeps completed text", () => {
    const itemId = "55555555-5555-4555-8555-555555555555";
    const nextId = "66666666-6666-4666-8666-666666666666";
    const payloads: AgentEventPayload[] = [
      {type:"phaseChanged",turnId,phase:"runningModel"},
      {type:"modelRequestStarted",turnId,attemptId:"77777777-7777-4777-8777-777777777777",model},
      {type:"itemStarted",turnId,itemId,kind:{type:"assistantMessage"}},
      {type:"textDelta",turnId,itemId,text:"中断的片段"},
      {type:"modelRequestFailed",turnId,attemptId:"77777777-7777-4777-8777-777777777777",error:"offline",retryable:true,category:"providerTemporaryUnavailable"},
    ];
    const interrupted = project(payloads);
    const pending = projectConversationMessages(interrupted);
    expect(pending[0]).toMatchObject({content:"中断的片段",reconnecting:true,streaming:false});
    expect(pending[0]?.reconnectHistory).toHaveLength(1);
    expect(() => project([...payloads,{type:"textDelta",turnId,itemId,text:"迟到的内容"}])).toThrow();
    payloads.push(
      {type:"modelRequestStarted",turnId,attemptId:"88888888-8888-4888-8888-888888888888",model},
      {type:"modelResponseStarted",turnId,attemptId:"88888888-8888-4888-8888-888888888888",responseId:"r2",model},
      {type:"itemStarted",turnId,itemId:nextId,kind:{type:"assistantMessage"}},
      {type:"textDelta",turnId,itemId:nextId,text:"重新生成的完整解释"},
    );
    expect(projectConversationMessages(project(payloads)).map(m => m.content)).toEqual(["重新生成的完整解释"]);
    payloads.push(
      {type:"itemCompleted",turnId,itemId:nextId},
      {type:"modelRequestFinished",turnId,attemptId:"88888888-8888-4888-8888-888888888888",inputTokens:null,outputTokens:null},
      {type:"turnFinished",turnId,outcome:"completed"},
    );
    const restored = project(payloads);
    expect(restored.turns.get(turnId)?.items.get(itemId)).toMatchObject({text:"中断的片段",completed:false,interrupted:true});
    expect(projectConversationMessages(restored).map(m => m.content)).toEqual(["重新生成的完整解释"]);
    expect(projectConversationMessages(restored)[0]?.reconnectHistory).toEqual(pending[0]?.reconnectHistory);
  });
  it("keeps an already displayed prefix until the regenerated sample catches up without joining divergent text", () => {
    const itemId = "55555555-5555-4555-8555-555555555555", nextId = "66666666-6666-4666-8666-666666666666";
    const payloads: AgentEventPayload[] = [
      {type:"phaseChanged",turnId,phase:"runningModel"},
      {type:"modelRequestStarted",turnId,attemptId:"77777777-7777-4777-8777-777777777777",model},
      {type:"itemStarted",turnId,itemId,kind:{type:"assistantMessage"}},
      {type:"textDelta",turnId,itemId,text:"这是已经显示过的正文"},
      {type:"modelRequestFailed",turnId,attemptId:"77777777-7777-4777-8777-777777777777",error:"offline",retryable:true,category:"providerTemporaryUnavailable"},
      {type:"modelRequestStarted",turnId,attemptId:"88888888-8888-4888-8888-888888888888",model},
      {type:"itemStarted",turnId,itemId:nextId,kind:{type:"assistantMessage"}},
    ];
    expect(projectConversationMessages(project(payloads)).map(m => m.content)).toEqual(["这是已经显示过的正文"]);
    payloads.push({type:"textDelta",turnId,itemId:nextId,text:"这是"});
    const restored = projectConversationMessages(project(payloads))[0]!;
    expect(restored.content).toBe("这是已经显示过的正文");
    expect(restored.rawData).toMatchObject({text:"这是"});
    expect(restored.streaming).toBe(true);
    expect(projectConversationMessages(project([...payloads,{type:"textDelta",turnId,itemId:nextId,text:"已经显示过的正文，以及新收到的字"}]))[0]?.content).toBe("这是已经显示过的正文，以及新收到的字");
    expect(projectConversationMessages(project([...payloads,{type:"textDelta",turnId,itemId:nextId,text:"另一种解释"}]))[0]?.content).toBe("这是另一种解释");
  });
  it("replays a non-model terminal failure as one visible error with its category", () => {
    const messages = projectConversationMessages(project([
      { type: "turnFailed", turnId, error: "run budget exhausted", category: "admission" },
      { type: "turnFinished", turnId, outcome: "failed" },
    ]));
    expect(messages).toHaveLength(1);
    expect(messages[0]).toMatchObject({ failure: true, turnOutcome: "failed" });
    expect(messages[0]?.content).toContain("本轮达到模型调用、工具调用或输出上限");
  });
  it("always shows a fallback for legacy failures without a model error", () => {
    const messages = projectConversationMessages(project([{ type: "turnFinished", turnId, outcome: "failed" }]));
    expect(messages[0]).toMatchObject({ failure: true });
    expect(messages[0]?.content).toContain("本轮未能完成");
  });
  it("does not show a terminal error before completion, on cancellation, or on success", () => {
    expect(projectConversationMessages(project([{ type: "turnFailed", turnId, error: "tool request rejected", category: "toolRound" }]))).toEqual([]);
    for (const outcome of ["completed", "canceled", "interrupted"] as const) {
      expect(projectConversationMessages(project([{ type: "turnFinished", turnId, outcome }]))).toEqual([]);
    }
  });
  it("rejects duplicate diagnostics, success after a diagnostic, and unsafe error text", () => {
    const diagnostic = { type: "turnFailed", turnId, error: "tool request rejected", category: "toolRound" } as const;
    expect(() => project([diagnostic, diagnostic])).toThrow();
    expect(() => project([diagnostic, { type: "turnFinished", turnId, outcome: "completed" }])).toThrow();
    for (const error of ["", " ", "x".repeat(1025), "a\nb"]) {
      expect(() => project([{ ...diagnostic, error }])).toThrow();
    }
  });
});
