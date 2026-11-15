import { describe, expect, it, vi } from "vitest";
import { applyAgentEvents, createAgentConversationProjection, decodeAgentEvent, type AgentHistoryResponse, type AgentHistoryWaitResponse, type AgentSessionResponse } from "@research-assistant/core/agent";
import type { AgentSessionGateway } from "@research-assistant/data/agent";
import { AgentConversationController } from "./conversationController";
import raw from "../../../../../../tests/fixtures/agent-events-v1.json";
const events = raw.map(decodeAgentEvent);
const id = "00000000-0000-0000-0000-000000000001";
const otherId = "00000000-0000-0000-0000-000000000099";
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function session(conversationId = id, watermark = 21): AgentSessionResponse { return { schemaVersion: 1, conversationId, watermark }; }
function page(afterSeq: number, end: number): AgentHistoryResponse {
  return { ...session(id, end), afterSeq, nextAfterSeq: null, events: events.slice(afterSeq, end) };
}
function setup() {
  const waiting = deferred<void>();
  const update = deferred<AgentHistoryWaitResponse>();
  const gateway: AgentSessionGateway = {
    create: vi.fn(async () => session()), open: vi.fn(async () => session()),
    readHistory: vi.fn(async () => page(0, 21)),
    waitForHistory: vi.fn(() => { waiting.resolve(); return update.promise; }),
  };
  return { gateway, waiting, update, controller: new AgentConversationController(gateway) };
}

describe("shared conversation controller", () => {
  it("waits for the history snapshot captured after open before becoming ready", async () => {
    const secondPage = deferred<AgentHistoryResponse>(), requested = deferred<void>(), waiting = deferred<void>();
    const gateway: AgentSessionGateway = {
      create: vi.fn(), open: vi.fn(async () => session(id, 3)),
      readHistory: vi.fn().mockResolvedValueOnce({ ...page(0, 3), watermark: 6, nextAfterSeq: 3 })
        .mockImplementationOnce(() => { requested.resolve(); return secondPage.promise; }),
      waitForHistory: vi.fn(() => { waiting.resolve(); return new Promise<AgentHistoryWaitResponse>(() => undefined); }),
    };
    const controller = new AgentConversationController(gateway);
    const ready: number[] = [];
    controller.subscribe(() => { const snapshot = controller.getSnapshot(); if (snapshot.status === "ready") ready.push(snapshot.projection!.seq); });
    try {
      await controller.open(id); await requested.promise;
      expect(controller.getSnapshot()).toMatchObject({ status: "syncing", projection: null });
      expect(ready).toEqual([]);
      secondPage.resolve({ ...page(3, 6), watermark: 6 });
      await waiting.promise;
      expect(ready).toEqual([6]);
    } finally { controller.dispose(); }
  });

  it("keeps the last complete projection while a live gap is resynchronized", async () => {
    const notice = deferred<AgentHistoryWaitResponse>(), firstWait = deferred<void>(), resyncPage = deferred<AgentHistoryResponse>();
    const resyncRead = deferred<void>(), secondWait = deferred<void>();
    const gateway: AgentSessionGateway = {
      create: vi.fn(), open: vi.fn(async () => session(id, 3)),
      readHistory: vi.fn().mockResolvedValueOnce(page(0, 3))
        .mockImplementationOnce(() => { resyncRead.resolve(); return resyncPage.promise; }),
      waitForHistory: vi.fn().mockImplementationOnce(() => { firstWait.resolve(); return notice.promise; })
        .mockImplementationOnce(() => { secondWait.resolve(); return new Promise<AgentHistoryWaitResponse>(() => undefined); }),
    };
    const controller = new AgentConversationController(gateway);
    try {
      await controller.open(id); await firstWait.promise;
      expect(controller.getSnapshot()).toMatchObject({ status: "ready", projection: { seq: 3 } });
      notice.resolve({ ...session(id, 6), afterSeq: 3, events: [], resyncRequired: true });
      await resyncRead.promise;
      expect(controller.getSnapshot()).toMatchObject({ status: "syncing", projection: { seq: 3 } });
      resyncPage.resolve(page(3, 6)); await secondWait.promise;
      expect(controller.getSnapshot()).toMatchObject({ status: "ready", projection: { seq: 6 } });
    } finally { controller.dispose(); }
  });

  it.each([false, true])("publishes a long completed history once, without replaying pages (cached=%s)", async cached => {
    const journal = [...events.slice(0, 17), ...Array.from({length: 220}, (_, index) => ({...events[17]!,
      payload: {...events[17]!.payload, text: `历史片段${index} `}})),
      events.find(event => event.payload.type === "itemCompleted" && event.payload.itemId === "00000000-0000-0000-0000-00000000000d")!,
      events.find(event => event.payload.type === "turnFinished")!]
      .map((event, index) => decodeAgentEvent({...event, seq: index + 1, timestampMs: index}));
    const checkpoint = cached ? applyAgentEvents(createAgentConversationProjection(id), journal.slice(0, 70)) : undefined;
    const finalPage = deferred<AgentHistoryResponse>(), requestedFinal = deferred<void>(), waiting = deferred<void>();
    const gateway: AgentSessionGateway = {
      create: vi.fn(async () => session(id, journal.length)), open: vi.fn(async () => session(id, journal.length)),
      readHistory: vi.fn(async request => {
        const end = Math.min(journal.length, request.afterSeq + request.limit);
        const result = {...session(id, journal.length), afterSeq: request.afterSeq, events: journal.slice(request.afterSeq, end),
          nextAfterSeq: end < journal.length ? end : null};
        if (end === journal.length) { requestedFinal.resolve(); return finalPage.promise; }
        return result;
      }),
      waitForHistory: vi.fn(() => { waiting.resolve(); return new Promise<AgentHistoryWaitResponse>(() => undefined); }),
    };
    const controller = new AgentConversationController(gateway, checkpoint);
    const visible: number[] = [];
    controller.subscribe(() => { const projection = controller.getSnapshot().projection; if (projection) visible.push(projection.seq); });
    try {
      await controller.open(id);
      await requestedFinal.promise;
      expect(controller.getSnapshot()).toMatchObject({status: "syncing", projection: null});
      expect(visible).toEqual([]);
      const afterSeq = vi.mocked(gateway.readHistory).mock.calls.at(-1)![0].afterSeq;
      finalPage.resolve({...session(id, journal.length), afterSeq, events: journal.slice(afterSeq), nextAfterSeq: null});
      await waiting.promise;
      expect(visible).toEqual([journal.length]);
      const projection = controller.getSnapshot().projection!;
      expect(projection.activeTurn).toBeNull();
      expect([...projection.turns.values()][0]?.outcome).toBe("completed");
      expect([...projection.turns.values()][0]?.items.get("00000000-0000-0000-0000-00000000000d")?.text).toContain("历史片段219");
    } finally { controller.dispose(); }
  });

  it("does not wait for a running turn to finish before showing its received snapshot and later deltas", async () => {
    const journal = events.slice(0, 18);
    const waiting = deferred<void>(), notice = deferred<AgentHistoryWaitResponse>(), updated = deferred<void>();
    const gateway: AgentSessionGateway = {
      create: vi.fn(async () => session(id, 18)), open: vi.fn(async () => session(id, 18)),
      readHistory: vi.fn(async request => ({...session(id, journal.length), afterSeq: request.afterSeq,
        events: journal.slice(request.afterSeq), nextAfterSeq: null})),
      waitForHistory: vi.fn(() => { waiting.resolve(); return notice.promise; }),
    };
    const controller = new AgentConversationController(gateway);
    controller.subscribe(() => { if (controller.getSnapshot().projection?.seq === 19) updated.resolve(); });
    try {
      await controller.open(id); await waiting.promise;
      expect(controller.getSnapshot()).toMatchObject({status: "ready", projection: {seq: 18}});
      expect(controller.getSnapshot().projection?.activeTurn).not.toBeNull();
      vi.mocked(gateway.waitForHistory).mockImplementation(() => new Promise(() => undefined));
      journal.push(decodeAgentEvent({...events[17]!, seq: 19, payload: {...events[17]!.payload, text: "新收到的增量"}}));
      notice.resolve({...session(id, 19), afterSeq: 18});
      await updated.promise;
      expect([...controller.getSnapshot().projection!.turns.values()][0]?.items.get("00000000-0000-0000-0000-00000000000d")?.text).toContain("新收到的增量");
    } finally { controller.dispose(); }
  });

  it("can retry an initial history read failure before any projection has been presented", async () => {
    const {controller, gateway, waiting} = setup();
    vi.mocked(gateway.readHistory).mockRejectedValueOnce(new Error("page failed"));
    const failed = deferred<void>();
    controller.subscribe(() => { if (controller.getSnapshot().status === "error") failed.resolve(); });
    try {
      await controller.open(id); await failed.promise;
      expect(controller.getSnapshot().projection).toBeNull();
      await controller.resume(); await waiting.promise;
      expect(controller.getSnapshot()).toMatchObject({status: "ready", projection: {seq: 21}});
    } finally { controller.dispose(); }
  });

  it("starts from a cached projection while catching up from its sequence", async () => {
    const cached = createAgentConversationProjection(id);
    const { controller, gateway } = setup();
    const resumed = new AgentConversationController(gateway, cached);
    expect(resumed.getSnapshot().status).toBe("paused");
    expect(resumed.getSnapshot().projection).toBe(cached);
    await resumed.open(id);
    expect(gateway.readHistory).toHaveBeenCalledWith({ conversationId: id, afterSeq: 0, limit: 100 });
    resumed.dispose(); controller.dispose();
  });
  it("opens, projects durable history and resumes without replaying accepted text", async () => {
    const { controller, gateway, waiting } = setup();
    const listener = vi.fn();
    const unsubscribe = controller.subscribe(listener);
    await controller.open(id, "create");
    await waiting.promise;
    const complete = controller.getSnapshot().projection;
    expect(controller.getSnapshot().status).toBe("ready");
    expect(complete?.seq).toBe(21);
    expect(gateway.create).toHaveBeenCalledWith(id);
    controller.pause();
    expect(controller.getSnapshot().status).toBe("paused");
    expect(controller.getSnapshot().projection).toBe(complete);
    vi.mocked(gateway.readHistory).mockResolvedValue(page(21, 21));
    await controller.resume();
    expect(gateway.readHistory).toHaveBeenLastCalledWith({ conversationId: id, afterSeq: 21, limit: 100 });
    expect(controller.getSnapshot().projection).toBe(complete);
    unsubscribe();
    const calls = listener.mock.calls.length;
    controller.dispose();
    expect(listener).toHaveBeenCalledTimes(calls);
  });

  it("switches identity immediately and ignores an earlier open completing last", async () => {
    const { controller, gateway } = setup();
    const old = deferred<AgentSessionResponse>();
    vi.mocked(gateway.open).mockImplementation((identity) => identity === id ? old.promise : Promise.resolve(session(otherId, 0)));
    vi.mocked(gateway.readHistory).mockResolvedValue({ ...session(otherId, 0), afterSeq: 0, events: [], nextAfterSeq: null });
    const first = controller.open(id);
    await controller.open(otherId);
    expect(controller.getSnapshot().projection?.conversationId).toBe(otherId);
    old.resolve(session());
    await first;
    expect(gateway.readHistory).toHaveBeenCalledTimes(1);
    expect(controller.getSnapshot().projection?.conversationId).toBe(otherId);
    controller.dispose();
  });

  it("keeps accepted history on failure and reconnects from that checkpoint", async () => {
    const { controller, gateway, waiting, update } = setup();
    const failed = deferred<void>();
    controller.subscribe(() => { if (controller.getSnapshot().status === "error") failed.resolve(); });
    await controller.open(id);
    await waiting.promise;
    update.reject(new Error("connection lost"));
    await failed.promise;
    expect(controller.getSnapshot().projection?.seq).toBe(21);
    expect(controller.getSnapshot().error).toBeInstanceOf(Error);
    vi.mocked(gateway.readHistory).mockResolvedValue(page(21, 21));
    vi.mocked(gateway.waitForHistory).mockImplementation(() => new Promise(() => undefined));
    await controller.resume();
    expect(controller.getSnapshot().status).toBe("ready");
    expect(controller.getSnapshot().error).toBeNull();
    controller.dispose();
  });

  it("disposal fences pending opens and reentrant switching cannot start an obsolete feed", async () => {
    const { controller, gateway } = setup();
    const pending = deferred<AgentSessionResponse>();
    vi.mocked(gateway.open).mockReturnValue(pending.promise);
    const opening = controller.open(id);
    controller.dispose();
    pending.resolve(session());
    await opening;
    expect(gateway.readHistory).not.toHaveBeenCalled();
    const next = new AgentConversationController(gateway);
    next.subscribe(() => { if (next.getSnapshot().status === "opening") next.pause(); });
    await next.open(id);
    expect(next.getSnapshot().status).toBe("paused");
    expect(gateway.open).toHaveBeenCalledTimes(1);
    next.dispose();
  });

  it("surfaces a bounded open timeout and ignores the late native response", async () => {
    vi.useFakeTimers();
    try {
      const { controller, gateway } = setup();
      const pending = deferred<AgentSessionResponse>();
      vi.mocked(gateway.open).mockReturnValue(pending.promise);

      const opening = controller.open(id);
      await vi.advanceTimersByTimeAsync(20_000);
      await opening;

      expect(controller.getSnapshot().status).toBe("error");
      expect(controller.getSnapshot().error).toBeInstanceOf(Error);
      expect(gateway.readHistory).not.toHaveBeenCalled();

      pending.resolve(session());
      await Promise.resolve();
      expect(controller.getSnapshot().status).toBe("error");
      expect(gateway.readHistory).not.toHaveBeenCalled();
      controller.dispose();
    } finally {
      vi.useRealTimers();
    }
  });
});
