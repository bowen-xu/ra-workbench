/** @vitest-environment happy-dom */
import {act} from "react";
import {createRoot, type Root} from "react-dom/client";
import {webcrypto} from "node:crypto";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {getAppStorage} from "@research-assistant/workbench-sdk/storage";
import {WritingEditorDiffDemo} from "./WritingEditorDiffDemo";
import type {WritingDiffDemoStorage} from "./WritingDiffDemoStorage";

type RuntimeFactory = typeof import("./writingDiffDemoRuntime").createWritingDiffDemoRuntime;
type Backend = {readonly runtime: ReturnType<RuntimeFactory>;
  readonly agent: Parameters<RuntimeFactory>[1]; readonly manuscriptId: string; readonly conversationId: string};
const io = vi.hoisted(() => ({backend: null as Backend | null,
  invoke: vi.fn(async () => {throw new Error("Unexpected native I/O");})}));
vi.mock("@tauri-apps/api/core", async importOriginal => ({
  ...await importOriginal<typeof import("@tauri-apps/api/core")>(), invoke: io.invoke, isTauri: () => true,
}));
vi.mock("./writingDiffDemoRuntime", async importOriginal => {
  const actual = await importOriginal<typeof import("./writingDiffDemoRuntime")>();
  return {...actual, createWritingDiffDemoRuntime: (...args: Parameters<RuntimeFactory>) => {
    const runtime = actual.createWritingDiffDemoRuntime(...args), storage = args[0] as WritingDiffDemoStorage;
    storage.ioDelayMs = 0;
    io.backend = {runtime, agent: args[1], manuscriptId: args[2], conversationId: args[3]};
    return runtime;
  }};
});
const nativeDigest = webcrypto.subtle.digest.bind(webcrypto.subtle);
(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT?: boolean}).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root, host: HTMLDivElement;
beforeEach(() => {
  getAppStorage().clear(); io.invoke.mockClear(); io.backend = null;
  vi.spyOn(crypto.subtle, "digest").mockImplementation((algorithm, data) =>
    nativeDigest(algorithm, new Uint8Array(data as ArrayBuffer)));
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove(); io.backend?.runtime.dispose();
  expect(io.invoke).not.toHaveBeenCalled(); vi.restoreAllMocks();
});
async function mount(): Promise<Backend> {
  await act(async () => root.render(<WritingEditorDiffDemo/>));
  const backend = io.backend!;
  await act(async () => backend.runtime.execution.reschedule(0));
  await vi.waitFor(async () => {
    await act(async () => Promise.resolve());
    expect(backend.agent.history.activeTurn(backend.conversationId)).toBeNull();
    const completions = [...backend.agent.snapshot().completions.values()];
    expect(completions.length).toBeGreaterThan(0);
    expect(completions.every(completion => completion.acknowledged)).toBe(true);
    expect(host.querySelector(".recovery-banner")).toBeNull();
    expect(host.textContent).not.toContain("请先等待或核对文件修改接收，再继续发送。");
    expect(host.querySelector<HTMLTextAreaElement>("textarea")?.disabled).toBe(false);
  }, {timeout: 8000});
  return backend;
}
async function reloadQueue(backend: Backend): Promise<void> {
  const lease = backend.runtime.gateways.queue.registry.acquire({manuscriptId: backend.manuscriptId, conversationId: backend.conversationId});
  if (!lease) throw new Error("The production Demo queue owner is unavailable");
  try {await act(async () => lease.controller.reload());} finally {lease.release();}
}
async function enqueue(backend: Backend): Promise<string> {
  const {runtime, manuscriptId, conversationId} = backend, inputId = crypto.randomUUID();
  const queue = await runtime.queue.gateway.read({manuscriptId, conversationId});
  const added = await runtime.queue.gateway.edit({manuscriptId, conversationId, requestId: crypto.randomUUID(), expectedRevision: queue.revision,
    edit: {type: "add", inputId, text: "/compact", userInput: []}});
  if (added.paused) await runtime.queue.gateway.edit({manuscriptId, conversationId, requestId: crypto.randomUUID(),
    expectedRevision: added.revision, edit: {type: "resumeInput", inputId}});
  await reloadQueue(backend); return inputId;
}

it("removes the sending mask only for the admitted queued compaction input and keeps its stop available", async () => {
  const backend = await mount(), {runtime, agent, manuscriptId, conversationId} = backend;
  runtime.setCompactionDuration(60_000);
  let releaseStart!: () => void;
  const startGate = new Promise<void>(resolve => {releaseStart = resolve;});
  const originalStart = runtime.compaction.gateway.start;
  const start = vi.spyOn(runtime.compaction.gateway, "start").mockImplementation(async request => {
    await startGate; return originalStart(request);
  });
  const settle = vi.spyOn(runtime.compaction.gateway, "settleWritingQueued");
  try {
    const inputId = await enqueue(backend);
    await vi.waitFor(() => expect(start).toHaveBeenCalledOnce());
    expect(agent.history.activeTurn(conversationId)).toBeNull();
    expect(host.querySelector(".agent-composer-state-overlay")).not.toBeNull();
    expect(host.querySelector('[aria-label="停止生成"]')).toBeNull();
    await act(async () => releaseStart());
    await act(async () => {await start.mock.results[0]!.value;});
    await vi.waitFor(() => expect(agent.history.activeTurn(conversationId)).not.toBeNull());
    expect(settle).not.toHaveBeenCalled();
    expect((await runtime.queue.gateway.read({manuscriptId, conversationId})).preparingInputId).toBe(inputId);
    await vi.waitFor(() => expect(host.querySelector(".agent-composer-state-overlay")).toBeNull());
    const stop = host.querySelector<HTMLButtonElement>('[aria-label="停止生成"]');
    expect(stop).not.toBeNull(); expect(stop?.disabled).toBe(false);
    const originalRead = runtime.queue.gateway.read, foreignInputId = crypto.randomUUID();
    // External I/O for an unrelated claim must retain its own recovery surface.
    vi.spyOn(runtime.queue.gateway, "read").mockImplementation(async scope => {
      const current = await originalRead(scope);
      return {...current, preparingInputId: foreignInputId,
        inputs: [...current.inputs, {inputId: foreignInputId, text: "Foreign input", userInput: []}]};
    });
    await reloadQueue(backend);
    await vi.waitFor(() => expect(host.querySelector(".agent-composer-state-overlay")).not.toBeNull());
    expect(host.querySelector('[aria-label="输入框准备状态"]')?.textContent).toBe("消息发送状态待确认");
    expect(start).toHaveBeenCalledOnce(); expect(settle).not.toHaveBeenCalled();
  } finally {releaseStart();}
});

it.each([false, true])("keeps a later ordinary claim covered after a terminal compaction reused the same input, pending settle ACK=%s", async pendingAck => {
  const backend = await mount(), {runtime, manuscriptId, conversationId} = backend;
  runtime.setCompactionDuration(0); runtime.compaction.nextOutcome = "failed";
  const scope = {manuscriptId, conversationId};
  let releaseAck!: () => void, nativeSettled = false;
  const ackGate = new Promise<void>(resolve => {releaseAck = resolve;});
  const originalSettle = runtime.compaction.gateway.settleWritingQueued;
  const compactStart = vi.spyOn(runtime.compaction.gateway, "start");
  const settle = vi.spyOn(runtime.compaction.gateway, "settleWritingQueued").mockImplementation(async request => {
    const result = await originalSettle(request);
    nativeSettled = true;
    if (pendingAck) await ackGate;
    return result;
  });
  const ordinaryStart = vi.spyOn(runtime.gateways.submission, "startWriting");
  try {
    const inputId = await enqueue(backend);
    await vi.waitFor(() => expect(settle).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(nativeSettled).toBe(true));
    const receiptKey = `research-assistant:writing-compaction-receipt:v1:${manuscriptId}:${conversationId}`;
    if (pendingAck) expect(getAppStorage().getItem(receiptKey)).not.toBeNull();
    else {
      await act(async () => {await settle.mock.results[0]!.value;});
      await vi.waitFor(() => expect(getAppStorage().getItem(receiptKey)).toBeNull());
    }
    expect(compactStart).toHaveBeenCalledOnce();
    let queue = await runtime.queue.gateway.read(scope);
    expect(queue.preparingInputId).toBeNull(); expect(queue.inputs.map(item => item.inputId)).toEqual([inputId]);
    const text = "Continue after failed compact", userInput = [{type: "text" as const, text, textElements: []}];
    queue = await runtime.queue.gateway.edit({...scope, requestId: crypto.randomUUID(), expectedRevision: queue.revision,
      edit: {type: "update", inputId, text, userInput}});
    queue = await runtime.queue.gateway.edit({...scope, requestId: crypto.randomUUID(), expectedRevision: queue.revision,
      edit: {type: "resumeInput", inputId}});
    const requestId = crypto.randomUUID();
    const prepared = await runtime.queue.preview.prepare({inputId, expectedRevision: queue.revision, submission: {
      conversationId, requestId, text, userInput, context: {manuscriptId, files: []},
      model: {providerId: "deepseek", modelId: "deepseek-v4-pro"}}});
    expect(prepared.preview.requestId).toBe(requestId);
    expect(requestId).not.toBe(compactStart.mock.calls[0]![0].requestId);
    await reloadQueue(backend);
    await vi.waitFor(() => expect(host.querySelector(".agent-composer-state-overlay")).not.toBeNull());
    expect(host.querySelector('[aria-label="输入框准备状态"]')?.textContent).toBe(pendingAck ? "发送中…" : "消息发送状态待确认");
    expect(host.querySelector<HTMLTextAreaElement>("textarea")?.disabled).toBe(true);
    expect(ordinaryStart).not.toHaveBeenCalled(); expect(compactStart).toHaveBeenCalledOnce(); expect(settle).toHaveBeenCalledOnce();
  } finally {
    await act(async () => {releaseAck(); if (settle.mock.results[0]) await settle.mock.results[0].value;});
  }
});
