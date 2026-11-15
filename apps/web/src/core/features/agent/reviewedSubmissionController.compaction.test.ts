import { expect, it, vi } from "vitest";
import type { AgentStartResponse } from "@research-assistant/core/agent";
import { ReviewedSubmissionController } from "./reviewedSubmissionController";

const receipt = { conversationId: "11111111-1111-4111-8111-111111111111", requestId: "22222222-2222-4222-8222-222222222222" };
const preview = { ...receipt, turnId: "33333333-3333-4333-8333-333333333333" };
const admission: AgentStartResponse = { ...preview, schemaVersion: 1, alreadyStarted: false, outcome: null };
class RetirementTestController extends ReviewedSubmissionController<typeof receipt, typeof receipt, typeof preview> {
  retireVerified(value: typeof preview): boolean {return this.retireUnstartedPreview(value);}
}
function setup() {
  const gateway = { prepare: vi.fn(async () => preview), restore: vi.fn(async () => preview), start: vi.fn(async () => admission) };
  const turns = { interrupt: vi.fn(async () => ({ schemaVersion: 1 as const, conversationId: receipt.conversationId,
    turnId: preview.turnId, cancellationRequested: true })) };
  const controller = new RetirementTestController({ conversationId: receipt.conversationId,
    decodeRequest: (value: typeof receipt) => structuredClone(value),
    decodeReceipt: () => ({ ...receipt }), receipt: (value: typeof receipt) => ({ ...value }),
  }, gateway, turns, { isStartRejected: error => !!error && typeof error === "object" && "code" in error && error.code === "historyChanged" });
  return { controller, gateway, turns };
}

it("preserves a definitive compact rejection and its original diagnostic", async () => {
  const { controller, gateway } = setup();
  const failure = { code: "historyChanged", reason: { expected: 3, actual: 4 } };
  gateway.start.mockRejectedValueOnce(failure);
  await controller.prepare(receipt); await controller.confirm("chatGptDefault");
  expect(controller.getSnapshot()).toMatchObject({ phase: "error", error: "startRejected", cause: failure, request: receipt });
  expect(controller.dismiss()).toBe(true);
  controller.dispose();
});

it("never downgrades an earlier unknown start after a later rejection", async () => {
  const { controller, gateway } = setup();
  gateway.start.mockRejectedValueOnce(new Error("acknowledgment lost"))
    .mockRejectedValueOnce({ code: "historyChanged", reason: { expected: 3, actual: 4 } });
  await controller.prepare(receipt); await controller.confirm("chatGptDefault");
  expect(controller.getSnapshot().error).toBe("start");
  await controller.confirm("chatGptDefault");
  expect(controller.getSnapshot()).toMatchObject({ error: "start", request: receipt });
  expect(controller.dismiss()).toBe(false);
  expect(gateway.start.mock.calls).toHaveLength(2);
  controller.dispose();
});

it("retains a terminal fact observed before the admission reply", async () => {
  const { controller, gateway } = setup();
  let finish!: (value: AgentStartResponse) => void;
  gateway.start.mockImplementationOnce(() => new Promise(resolve => { finish = resolve; }));
  await controller.prepare(receipt);
  const starting = controller.confirm("chatGptDefault");
  expect(controller.getSnapshot().phase).toBe("starting");
  controller.observeOutcome(preview.turnId, "completed");
  finish(admission); await starting;
  expect(controller.getSnapshot()).toMatchObject({ phase: "started", outcome: "completed" });
  controller.dispose();
});

it("keeps a stop acknowledgement distinct from a durable terminal outcome", async () => {
  const { controller, turns } = setup();
  await controller.prepare(receipt); await controller.confirm("chatGptDefault"); await controller.stop();
  expect(turns.interrupt).toHaveBeenCalledWith({ conversationId: receipt.conversationId, turnId: preview.turnId });
  expect(controller.getSnapshot()).toMatchObject({ outcome: null, stopRequested: true });
  controller.observeOutcome(preview.turnId, "canceled");
  expect(controller.getSnapshot()).toMatchObject({ outcome: "canceled", stopRequested: false, stopPending: false });
  controller.dispose();
});

it("restores preparation using the same receipt after a lost response", async () => {
  const { controller, gateway } = setup();
  const original = { ...receipt };
  gateway.prepare.mockRejectedValueOnce(new Error("prepare reply lost"));
  await controller.prepare(original);
  original.requestId = "44444444-4444-4444-8444-444444444444";
  expect(controller.getSnapshot()).toMatchObject({ error: "prepare", request: receipt });
  await controller.retryPrepare(true);
  expect(gateway.restore).toHaveBeenCalledWith(receipt);
  expect(gateway.prepare).toHaveBeenCalledTimes(1);
  controller.dispose();
});

it("observes an exact durable start without fabricating an admission acknowledgement", async () => {
  const { controller, gateway } = setup(), acknowledged = vi.fn();
  await controller.prepare(receipt, acknowledged);
  controller.observeStarted("44444444-4444-4444-8444-444444444444");
  expect(controller.getSnapshot().phase).toBe("preview");
  expect(acknowledged).not.toHaveBeenCalled();
  controller.observeStarted(preview.turnId);
  expect(controller.getSnapshot()).toMatchObject({ phase: "started", admission: null, outcome: null });
  controller.observeStarted(preview.turnId);
  controller.observeOutcome(preview.turnId, "completed");
  await controller.confirm("chatGptDefault");
  expect(acknowledged).toHaveBeenCalledTimes(1);
  expect(gateway.start).not.toHaveBeenCalled();
  controller.dispose();
});

it("clears the exact unstarted preview only after a domain's verified retirement without acknowledging its draft", async () => {
  const {controller, gateway} = setup(), acknowledged = vi.fn();
  gateway.start.mockRejectedValueOnce(new Error("unconfirmed start"));
  await controller.prepare(receipt, acknowledged); await controller.confirm("chatGptDefault");
  expect(controller.dismiss()).toBe(false);
  expect(controller.retireVerified({...preview})).toBe(false);
  expect(controller.retireVerified(preview)).toBe(true);
  expect(controller.getSnapshot()).toMatchObject({phase: "idle", outcome: null, admission: null, preview: null});
  expect(acknowledged).not.toHaveBeenCalled(); controller.dispose();
});

it.each(["acknowledged", "history", "terminal", "disposed"] as const)("refuses retirement after %s evidence", async evidence => {
  const {controller} = setup(); await controller.prepare(receipt);
  if (evidence === "acknowledged") await controller.confirm("chatGptDefault");
  if (evidence === "history" || evidence === "terminal") controller.observeStarted(preview.turnId);
  if (evidence === "terminal") controller.observeOutcome(preview.turnId, "completed");
  if (evidence === "disposed") controller.dispose();
  const original = controller.getSnapshot();
  expect(controller.retireVerified(preview)).toBe(false); expect(controller.getSnapshot()).toBe(original); controller.dispose();
});

it("retains the in-flight original start when a domain attempts retirement", async () => {
  const {controller, gateway} = setup(); let release!: (value: AgentStartResponse) => void;
  gateway.start.mockImplementationOnce(() => new Promise(resolve => {release = resolve;}));
  await controller.prepare(receipt); const pending = controller.confirm("chatGptDefault");
  expect(controller.retireVerified(preview)).toBe(false);
  release(admission); await pending;
  expect(controller.getSnapshot()).toMatchObject({phase: "started", admission}); controller.dispose();
});
