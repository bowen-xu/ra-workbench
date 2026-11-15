/** @vitest-environment happy-dom */
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, it, vi } from "vitest";
import { applyAgentEvents, createAgentConversationProjection, type AgentEventPayload } from "@research-assistant/core/agent";
import { WritingAgentTranscript } from "./presentation/WritingAgentTranscript";
import { projectConversationMessages } from "./projectConversationMessages";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
it.each([undefined, "finalAnswer"] as const)("keeps matching retry text mounted and animates only divergence (%s)", async phase => {
  const turnId = crypto.randomUUID(), conversationId = crypto.randomUUID(), requestId = crypto.randomUUID();
  const model = {providerId:"mock",modelId:"mock",revision:null};
  let projection = createAgentConversationProjection(conversationId), seq = 0;
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const descriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype,"animate");
  const animate = vi.fn(()=>({cancel:vi.fn()}) as unknown as Animation);
  Object.defineProperty(HTMLElement.prototype,"animate",{configurable:true,value:animate});
  const render = async (...payloads: AgentEventPayload[]) => {
    projection = applyAgentEvents(projection,payloads.map(payload=>({schemaVersion:1,conversationId,requestId,seq:++seq,timestampMs:seq,payload})));
    await act(async ()=>root.render(<WritingAgentTranscript messages={projectConversationMessages(projection)} isRunning />));
  };
  const start = async (attemptId: string, itemId: string, text: string) => render(
    {type:"modelRequestStarted",turnId,attemptId,model},
    {type:"modelResponseStarted",turnId,attemptId,responseId:`response-${attemptId}`,model},
    {type:"itemStarted",turnId,itemId,kind:{type:"assistantMessage"}},
    {type:"textDelta",turnId,itemId,text},
  );
  const fail = async (attemptId: string) => render({type:"modelRequestFailed",turnId,attemptId,error:"offline",retryable:true,category:null});
  try {
    await render({type:"turnStarted",turnId,contextSnapshotId:crypto.randomUUID(),model}, {type:"phaseChanged",turnId,phase:"runningModel"});
    const first = crypto.randomUUID(), initialAttempt = crypto.randomUUID();
    await start(initialAttempt,first,"已经显示的正文");
    if (phase) await render({type:"itemPhaseChanged",turnId,itemId:first,phase});
    const body = host.querySelector('.writing-agent-markdown')!;
    const glyph = body.querySelector('.stream-reveal-glyph');
    animate.mockClear();
    await fail(initialAttempt);
    const second = crypto.randomUUID(), retry = crypto.randomUUID();
    await start(retry,second,""); // Identity/phase events may arrive before text.
    expect(host.querySelector('.writing-agent-markdown')).toBe(body);
    await render({type:"textDelta",turnId,itemId:second,text:"已经"});
    expect(body.textContent).toBe("已经显示的正文");
    expect(body.querySelector('.stream-reveal-glyph')).toBe(glyph);
    await render({type:"textDelta",turnId,itemId:second,text:"显示的正文，加上新的尾部"});
    await vi.waitFor(() => expect(body.textContent).toBe("已经显示的正文，加上新的尾部"));
    expect(animate).not.toHaveBeenCalled();
    expect(host.querySelector('article')?.dataset.agentMessageId).toBe(`${turnId}/${second}`);
    await fail(retry);
    const third = crypto.randomUUID(), divergentAttempt = crypto.randomUUID();
    await start(divergentAttempt,third,"另一份解释");
    expect(host.querySelector('.writing-agent-markdown')).toBe(body);
    await vi.waitFor(() => expect(body.textContent).toBe("另一份解释"));
    expect(animate).toHaveBeenCalledTimes(1);
    expect(animate).toHaveBeenLastCalledWith([{opacity:0},{opacity:1}],{duration:300,easing:"ease-out"});
    // An older matching draft must not reappear after the intervening divergence.
    await fail(divergentAttempt);
    await start(crypto.randomUUID(),crypto.randomUUID(),"已经");
    await vi.waitFor(() => expect(body.textContent).toBe("已经"));
    expect(animate).toHaveBeenCalledTimes(2);
  } finally {
    await act(async ()=>root.unmount()); host.remove();
    if(descriptor) Object.defineProperty(HTMLElement.prototype,"animate",descriptor);
    else Reflect.deleteProperty(HTMLElement.prototype,"animate");
  }
});

async function finalView() {
  const turnId = crypto.randomUUID(), conversationId = crypto.randomUUID(), requestId = crypto.randomUUID();
  const model = {providerId: "mock", modelId: "mock", revision: null};
  let projection = createAgentConversationProjection(conversationId), seq = 0;
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  async function render(...payloads: AgentEventPayload[]) {
    projection = applyAgentEvents(projection, payloads.map(payload => ({schemaVersion: 1, conversationId, requestId, seq: ++seq, timestampMs: seq, payload})));
    await act(async () => root.render(<WritingAgentTranscript messages={projectConversationMessages(projection)} isRunning />));
  }
  async function startAttempt() {
    const attemptId = crypto.randomUUID();
    await render({type: "modelRequestStarted", turnId, attemptId, model},
      {type: "modelResponseStarted", turnId, attemptId, responseId: `response-${attemptId}`, model}); return attemptId;
  }
  async function final(text: string, completed = false) {
    const itemId = crypto.randomUUID();
    await render({type: "itemStarted", turnId, itemId, kind: {type: "assistantMessage"}},
      {type: "textDelta", turnId, itemId, text}, {type: "itemPhaseChanged", turnId, itemId, phase: "finalAnswer"},
      ...(completed ? [{type: "itemCompleted" as const, turnId, itemId}] : []));
    return itemId;
  }
  const body = (itemId: string) => host.querySelector(`[data-agent-message-id="${turnId}/${itemId}"] .writing-agent-markdown`)!;
  const fail = (attemptId: string) => render({type: "modelRequestFailed", turnId, attemptId, error: "offline", retryable: true, category: null});
  await render({type: "turnStarted", turnId, contextSnapshotId: crypto.randomUUID(), model}, {type: "phaseChanged", turnId, phase: "runningModel"});
  return {host, turnId, model, render, startAttempt, final, body, fail, messages: () => projectConversationMessages(projection),
    close: async () => { await act(async () => root.unmount()); host.remove(); }};
}

it("retains independent completed finals while a later interrupted final retries in its own display slot", async () => {
  const view = await finalView();
  try {
    const firstAttempt = await view.startAttempt();
    const completed = await view.final("已经完成的结论", true), draft = await view.final("重试中的第二段");
    const completedBody = view.body(completed), draftBody = view.body(draft);
    expect(completedBody).not.toBe(draftBody);
    const keys = view.messages().map(message => message.renderKey); expect(new Set(keys).size).toBe(2);
    await view.fail(firstAttempt); await view.startAttempt(); const retry = await view.final("重试中的");
    expect(view.body(completed)).toBe(completedBody); expect(view.body(retry)).toBe(draftBody);
    expect(draftBody.textContent).toBe("重试中的第二段"); expect(view.host.querySelectorAll(".writing-agent-markdown")).toHaveLength(2);
    expect(view.messages().map(message => message.id)).toEqual([`${view.turnId}/${completed}`, `${view.turnId}/${retry}`]);
    await view.render({type: "textDelta", turnId: view.turnId, itemId: retry, text: "第二段的新结尾"}, {type: "itemCompleted", turnId: view.turnId, itemId: retry});
    expect(view.body(retry)).toBe(draftBody); expect(draftBody.textContent).toBe("重试中的第二段的新结尾");
    expect(completedBody.closest("details")).toBeNull(); expect(draftBody.closest("details")).toBeNull();
  } finally { await view.close(); }
});

it("does not merge different messages in the same failed attempt, even when their text is identical", async () => {
  const view = await finalView();
  try {
    const attempt = await view.startAttempt(), first = await view.final("相同的文字"), second = await view.final("相同的文字");
    const firstBody = view.body(first), secondBody = view.body(second);
    expect(firstBody).not.toBe(secondBody);
    await view.fail(attempt);
    expect(view.body(first)).toBe(firstBody); expect(view.body(second)).toBe(secondBody);
    expect(view.host.querySelectorAll(".writing-agent-markdown")).toHaveLength(2);
    expect(new Set(view.messages().map(message => message.renderKey)).size).toBe(2);
    expect(view.messages().every(message => message.interrupted)).toBe(true);
  } finally { await view.close(); }
});

it("does not reuse a completed final's slot for another attempt with the same text", async () => {
  const view = await finalView();
  try {
    const attempt = await view.startAttempt(), first = await view.final("相同的文字", true), firstBody = view.body(first);
    await view.render({type: "modelRequestFinished", turnId: view.turnId, attemptId: attempt, inputTokens: null, outputTokens: null});
    await view.startAttempt(); const second = await view.final("相同的文字");
    expect(view.body(first)).toBe(firstBody); expect(view.body(second)).not.toBe(firstBody);
    expect(view.host.querySelectorAll(".writing-agent-markdown")).toHaveLength(2);
    expect(new Set(view.messages().map(message => message.renderKey)).size).toBe(2);
  } finally { await view.close(); }
});

it.each(["canceled", "interrupted"] as const)("keeps a partial final and its slot when retry ends %s", async outcome => {
  const view = await finalView();
  try {
    const attempt = await view.startAttempt(), first = await view.final("已经显示的正文"), body = view.body(first);
    await view.fail(attempt); await view.startAttempt(); const second = await view.final("已经");
    expect(view.body(second)).toBe(body);
    await view.render({type: "turnFinished", turnId: view.turnId, outcome});
    expect(view.body(second)).toBe(body); expect(body.textContent).toBe("已经显示的正文\n（未完成）");
    expect(view.host.querySelectorAll(".writing-agent-markdown")).toHaveLength(1);
    expect(view.host.querySelector('[data-agent-activity="aborted"]')).not.toBeNull();
    expect(view.messages()[0]).toMatchObject({turnOutcome: outcome, interrupted: true, streaming: false});
  } finally { await view.close(); }
});
