// @vitest-environment happy-dom
import { expect, it, vi } from "vitest";
import { type AgentContextPreview } from "@research-assistant/core/agent";
import { MockIndependentAgents } from "./MockIndependentAgents";
import { MockConversationQueue } from "./MockConversationQueue";

it("leaves naming to the shared client without truncating messages or replacing a manual title", async () => {
  vi.useFakeTimers();
  const values = new Map<string, string>();
  const storage: Storage = {
    get length() { return values.size; }, key: index => [...values.keys()][index] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); },
    removeItem: key => { values.delete(key); }, clear: () => values.clear(),
  };
  const conversationId = crypto.randomUUID();
  const text = "请用中文解释下面的 PDF 选段，说明关键术语和推理；缺少上下文时请明确指出。\n\n文献：Synthetic Paper\n所选原文：Full selected passage.";
  const preview = (input: string): AgentContextPreview => ({ schemaVersion: 1, conversationId,
    requestId: crypto.randomUUID(), turnId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID(),
    input: { text: input, model: { providerId: "fixture", modelId: "mock", revision: null }, reasoningEffort: null, context: [] },
  });
  const initial = preview(text);
  const userInput: readonly import("@research-assistant/core/agent").AgentUserInput[] = [
    { type: "text", text, textElements: [] },
    { type: "mention", name: "Synthetic Paper", target: { kind: "file", sourceId: "paper", resourceId: "paper", revision: "v1" } },
  ];
  const first = { ...initial, input: { ...initial.input, userInput } }, second = preview("后续问题");
  storage.setItem("independent-submissions", JSON.stringify([first, second].map(value => [value.requestId, value])));
  const agents = new MockIndependentAgents(storage, { delayMs: 0, nextFailure: null,
    beforeOperation: async () => undefined, configure: () => undefined, reset: () => undefined, dispose: () => undefined });
  try {
    await agents.gateway.sessions.create(conversationId);
    await agents.gateway.start(first);
    const generated = await agents.metadata.read({ id: conversationId });
    expect(generated.title).toBe("新对话");
    const history = await agents.gateway.sessions.readHistory({ conversationId, afterSeq: 0, limit: 100 });
    expect(history.events.some(event => event.payload.type === "textDelta" && event.payload.text === text)).toBe(true);
    const user = history.events.find(event => event.payload.type === "itemStarted" && event.payload.kind.type === "userMessage")!.payload;
    if (user.type !== "itemStarted") throw new Error("missing user item");
    const metadata = agents.inspectMessage({ conversationId, turnId: first.turnId, itemId: user.itemId, referencesOnly: true });
    expect(metadata.userInput).toEqual(userInput);
    expect(metadata.frozenSubmission).toBeNull();
    await vi.runAllTimersAsync();
    await agents.catalog.rename({ operationId: crypto.randomUUID(), id: conversationId, expectedTitle: generated.title, title: "自定标题" });
    await agents.gateway.start(second);
    expect((await agents.metadata.read({ id: conversationId })).title).toBe("自定标题");
  } finally { agents.dispose(); vi.useRealTimers(); }
});

it("admits a staged mock response before completion and interrupts its original execution owner", async () => {
  vi.useFakeTimers();
  const values = new Map<string, string>();
  const storage: Storage = { get length() { return values.size; }, key: index => [...values.keys()][index] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); }, removeItem: key => { values.delete(key); }, clear: () => values.clear() };
  const preview: AgentContextPreview = { schemaVersion: 1, conversationId: crypto.randomUUID(), requestId: crypto.randomUUID(),
    turnId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID(), input: { text: "修改笔记", model: { providerId: "fixture", modelId: "mock", revision: null }, reasoningEffort: null, context: [] } };
  storage.setItem("independent-submissions", JSON.stringify([[preview.requestId, preview]]));
  let signal: AbortSignal | undefined;
  const run = vi.fn(async (_preview, _history, _admission, cancellation: AbortSignal) => {
    signal = cancellation;
    await new Promise<void>(resolve => cancellation.addEventListener("abort", () => resolve(), { once: true }));
  });
  const agents = new MockIndependentAgents(storage, { delayMs: 0, nextFailure: null, beforeOperation: async () => undefined,
    configure() {}, reset() {}, dispose() {} }, { supports: () => true, run });
  try {
    await agents.gateway.sessions.create(preview.conversationId);
    const result = await agents.gateway.start(preview);
    expect(result.outcome).toBeNull();
    await vi.advanceTimersByTimeAsync(100);
    expect(run).toHaveBeenCalledTimes(1);
    await agents.gateway.interrupt(preview.conversationId, preview.turnId);
    await vi.runAllTimersAsync();
    expect(signal?.aborted).toBe(true);
    const history = await agents.gateway.sessions.readHistory({ conversationId: preview.conversationId, afterSeq: 0, limit: 100 });
    expect(history.events.filter(event => event.payload.type === "turnFinished").map(event => event.payload)).toEqual([
      { type: "turnFinished", turnId: preview.turnId, outcome: "canceled" },
    ]);
  } finally { agents.dispose(); vi.useRealTimers(); }
});

it("does not start a previously admitted queue message while its current queue is paused", async () => {
  vi.useFakeTimers();
  const values = new Map<string, string>();
  const storage: Storage = {get length() {return values.size;}, key: index => [...values.keys()][index] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => {values.set(key, value);},
    removeItem: key => {values.delete(key);}, clear: () => values.clear()};
  const conversationId = crypto.randomUUID(), inputId = crypto.randomUUID();
  const preview: AgentContextPreview = {schemaVersion: 1, conversationId, requestId: crypto.randomUUID(), turnId: crypto.randomUUID(),
    contextSnapshotId: crypto.randomUUID(), input: {text: "queued", model: {providerId: "fixture", modelId: "mock", revision: null}, reasoningEffort: null, context: []}};
  const queue = new MockConversationQueue(storage, async () => undefined);
  await queue.gateway.edit({conversationId, requestId: crypto.randomUUID(), expectedRevision: 0, edit: {type: "add", inputId, text: "queued"}});
  const frozen = {...preview, queue: queue.claim(preview, {inputId, expectedRevision: 1})};
  queue.admit(frozen);
  storage.setItem("independent-submissions", JSON.stringify([[frozen.requestId, frozen]]));
  const agents = new MockIndependentAgents(storage, {delayMs: 0, nextFailure: null, beforeOperation: async () => undefined,
    configure() {}, reset() {}, dispose() {}});
  try {
    await agents.gateway.sessions.create(conversationId);
    await expect(agents.gateway.start(frozen, {resumeQueue: true})).rejects.toBe("confirmationMismatch");
    expect((await agents.gateway.sessions.readHistory({conversationId, afterSeq: 0, limit: 100})).events).toEqual([]);
    const paused = await agents.gateway.queue!.read({conversationId});
    expect(paused.paused).toBe(true);
    await agents.gateway.queue!.edit({conversationId, requestId: crypto.randomUUID(), expectedRevision: paused.revision, edit: {type: "resume"}});
    await agents.gateway.start(frozen, {resumeQueue: true});
    const history = await agents.gateway.sessions.readHistory({conversationId, afterSeq: 0, limit: 100});
    expect(history.events.filter(event => event.payload.type === "turnStarted")).toHaveLength(1);
  } finally {agents.dispose(); vi.useRealTimers();}
});
