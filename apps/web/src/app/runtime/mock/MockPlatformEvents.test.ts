// @vitest-environment happy-dom
import {afterEach, expect, it, vi} from "vitest";
import {MockPlatformEvents} from "./MockPlatformEvents";
import {getAppRuntime, selectAppRuntime} from "../../../core/runtime/appRuntime";
import {writingDeliveryToolGateway} from "../../../core/public/agentFileDelivery";
import {writingSuccessorToolGateway} from "../../../core/public/agentSuccessorWorkspace";
const previous = getAppRuntime();
afterEach(() => selectAppRuntime(previous));
const deliveryEvent = "research-writing-file-delivery";

it("delivers isolated events and supports idempotent unsubscribe and disposal", async () => {
  const events = new MockPlatformEvents(), other = new MockPlatformEvents(), receive = vi.fn(), foreign = vi.fn();
  const release = await events.listen(deliveryEvent, receive);
  await other.listen(deliveryEvent, foreign);
  events.emit(deliveryEvent, {value: 1});
  expect(receive).toHaveBeenCalledWith({event: deliveryEvent, id: expect.any(Number), payload: {value: 1}});
  expect(foreign).not.toHaveBeenCalled();
  release(); await events.listen(deliveryEvent, receive); release();
  events.emit(deliveryEvent, {value: 2}); expect(receive).toHaveBeenCalledTimes(2);
  events.dispose();
  await expect(events.listen(deliveryEvent, receive)).rejects.toThrow("MOCK_PLATFORM_EVENTS_DISPOSED");
  expect(() => events.emit(deliveryEvent, {})).toThrow("MOCK_PLATFORM_EVENTS_DISPOSED");
  other.dispose();
});

it("keeps unsupported events unavailable rather than silently ignoring them", async () => {
  const events = new MockPlatformEvents();
  await expect(events.listen("unsupported", vi.fn())).rejects.toThrow("预览未覆盖平台事件");
  expect(() => events.emit("unsupported", {})).toThrow("预览未覆盖平台事件");
  events.dispose();
});

it("routes writing events through the production listener's identity decoders", async () => {
  const events = new MockPlatformEvents();
  selectAppRuntime({mode: "preview", storage: previous.storage, gateways: {}, events, dispose() {events.dispose();}});
  const scope = {manuscriptId: crypto.randomUUID(), conversationId: crypto.randomUUID(), requestId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID()};
  const turnId = crypto.randomUUID(), requestId = crypto.randomUUID();
  const request = {scope, turnId, requestId, binding: {scope, turnId, leaseId: crypto.randomUUID(), manifestHash: "a".repeat(64)}};
  const receive = vi.fn(), successor = vi.fn();
  const release = await writingDeliveryToolGateway.listen(receive), releaseSuccessor = await writingSuccessorToolGateway.listen(successor);
  events.emit(deliveryEvent, request); expect(receive).toHaveBeenCalledExactlyOnceWith(request);
  events.emit(deliveryEvent, {...request, binding: {...request.binding, turnId: crypto.randomUUID()}});
  expect(receive).toHaveBeenCalledTimes(1);
  const next = {scope, turnId, requestId: crypto.randomUUID(), leaseId: crypto.randomUUID(), predecessorDeliveryId: crypto.randomUUID(), generation: 1};
  events.emit("research-writing-successor-workspace", next); expect(successor).toHaveBeenCalledExactlyOnceWith(next);
  release(); releaseSuccessor(); events.emit(deliveryEvent, request); expect(receive).toHaveBeenCalledTimes(1);
  events.dispose();
});
