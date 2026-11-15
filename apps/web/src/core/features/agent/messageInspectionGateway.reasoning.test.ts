import { expect, it, vi } from "vitest";
import { readMessageInspection } from "./messageInspectionGateway";

const port = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("../../runtime/platformCore", () => ({ invoke: port.invoke }));
vi.mock("../../runtime/appRuntime", () => ({ hasPlatformCapabilities: () => true }));
const conversationId = "11111111-1111-4111-8111-111111111111", turnId = "22222222-2222-4222-8222-222222222222", itemId = "33333333-3333-4333-8333-333333333333";
it("reads exact public-summary progress and completion through the production inspector decoder", async () => {
  const payloads = [{ type: "itemStarted", turnId, itemId, kind: { type: "reasoningSummary", parts: [] } },
    { type: "reasoningSummaryDelta", turnId, itemId, summaryIndex: "9223372036854775807", text: "draft" },
    { type: "itemAccepted", turnId, itemId, kind: { type: "reasoningSummary", parts: [{ index: "0", startByte: 0, lenByte: 3 }] }, text: "思", phase: null, source: null }];
  const events = payloads.map((payload, index) => ({ schemaVersion: 1, conversationId, requestId: turnId, seq: index + 1, timestampMs: 1, payload }));
  port.invoke.mockResolvedValue({ schemaVersion: 1, conversationId, turnId, itemId, events, frozenSubmission: null });
  const request = { conversationId, turnId, itemId };
  expect((await readMessageInspection(request)).events).toEqual(events);
  expect(port.invoke).toHaveBeenCalledExactlyOnceWith("agent_read_message_data", { request });
  port.invoke.mockResolvedValue({ schemaVersion: 1, conversationId, turnId, itemId, events: [{ ...events[1], payload: { ...payloads[1], summaryIndex: 9223372036854776000 } }], frozenSubmission: null });
  await expect(readMessageInspection(request)).rejects.toThrow();
});
