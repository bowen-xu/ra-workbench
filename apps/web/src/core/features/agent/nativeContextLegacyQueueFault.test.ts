import {expect, it, vi} from "vitest";
import type {AgentContextGateway} from "../../services/agentContextGateway";
import type {NativeAgentPreview} from "../../runtime/nativeAgentPreview";
import {nativeContextLegacyQueueFault} from "./nativeContextLegacyQueueFault";

const id = (n: number) => `aaaaaaaa-aaaa-4aaa-8aaa-${String(n).padStart(12, "0")}`;
const ordinary = {conversationId: id(1), requestId: id(2), contextSnapshotId: id(3), queue: null, resumeQueue: false};
const queued = {...ordinary, queue: {inputId: id(4), expectedRevision: 2}};
it("starts the original queued turn once before preparing legacy evidence and losing its reply", async () => {
  const response = {schemaVersion: 1, conversationId: id(1), requestId: id(2), turnId: id(6), runId: id(7), alreadyStarted: false, outcome: null};
  const start = vi.fn(async () => response), read = vi.fn(), queue = {read: vi.fn(), edit: vi.fn()};
  const invoke = vi.fn(async () => ({schemaVersion: 1, prepared: true}));
  const gateway = {guardedStart: {start, read}, queue} as unknown as AgentContextGateway;
  let armed = true; const finished = vi.fn();
  const wrapped = nativeContextLegacyQueueFault(gateway, {invoke} as unknown as NativeAgentPreview, () => {const value = armed; armed = false; return value ? "settled" : false;}, finished);
  expect(wrapped.queue).toBe(queue); expect(wrapped.guardedStart!.read).toBe(read);
  await expect(wrapped.guardedStart!.start(ordinary, id(5))).resolves.toEqual(response); expect(armed).toBe(true);
  await expect(wrapped.guardedStart!.start(queued, id(5))).rejects.toEqual({code: "AGENT_PREVIEW_SYNTHETIC_LEGACY_START_RESPONSE_LOST"});
  expect(invoke).toHaveBeenCalledExactlyOnceWith("agent_preview_forget_settled_context_queue_admission", {request: queued, expectedEpoch: id(5), runId: id(7)});
  expect(start).toHaveBeenCalledTimes(2);
  expect(finished).toHaveBeenCalledOnce();
  await expect(wrapped.guardedStart!.start(queued, id(5))).resolves.toEqual(response);
});
it("preserves the real fixture error after the worker was started", async () => {
  const start = vi.fn(async () => ({runId: id(7)})), cause = {code: "AGENT_PREVIEW_TASK_NOT_SETTLED"};
  const gateway = {guardedStart: {start, read: vi.fn()}} as unknown as AgentContextGateway;
  const wrapped = nativeContextLegacyQueueFault(gateway, {invoke: vi.fn().mockRejectedValue(cause)} as unknown as NativeAgentPreview, () => "settled");
  await expect(wrapped.guardedStart!.start(queued, id(5))).rejects.toBe(cause); expect(start).toHaveBeenCalledOnce();
});
it("keeps the original start pending until the native fixture finishes", async () => {
  let release!: (value: {schemaVersion: number; prepared: boolean}) => void;
  const pending = new Promise<{schemaVersion: number; prepared: boolean}>(resolve => {release = resolve;});
  const invoke = vi.fn(() => pending), finished = vi.fn();
  const start = vi.fn(async () => ({runId: id(7)}));
  const gateway = {guardedStart: {start, read: vi.fn()}} as unknown as AgentContextGateway;
  const wrapped = nativeContextLegacyQueueFault(gateway, {invoke} as unknown as NativeAgentPreview, () => "settled", finished);
  const outcome = wrapped.guardedStart!.start(queued, id(5));
  await vi.waitFor(() => expect(invoke).toHaveBeenCalledOnce());
  expect(finished).not.toHaveBeenCalled();
  release({schemaVersion: 1, prepared: true});
  await expect(outcome).rejects.toEqual({code: "AGENT_PREVIEW_SYNTHETIC_LEGACY_START_RESPONSE_LOST"});
  expect(finished).toHaveBeenCalledOnce();
});
it("prepares running legacy evidence before losing the original reply", async () => {
  const start = vi.fn(async () => ({runId: id(7)})), invoke = vi.fn(async () => ({schemaVersion: 1, prepared: true}));
  const gateway = {guardedStart: {start, read: vi.fn()}} as unknown as AgentContextGateway;
  const wrapped = nativeContextLegacyQueueFault(gateway, {invoke} as unknown as NativeAgentPreview, () => "running");
  await expect(wrapped.guardedStart!.start(queued, id(5))).rejects.toEqual({code: "AGENT_PREVIEW_SYNTHETIC_LEGACY_START_RESPONSE_LOST"});
  expect(invoke).toHaveBeenCalledExactlyOnceWith("agent_preview_forget_running_context_queue_admission", {request: queued, expectedEpoch: id(5), runId: id(7)});
  expect(start).toHaveBeenCalledOnce();
});
