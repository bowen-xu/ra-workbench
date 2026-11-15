import { describe, expect, it, vi } from "vitest";
import {
  applyAgentEvents, createAgentConversationProjection, decodeAgentEvent,
  type AgentHistoryResponse, type AgentHistoryWaitResponse, type AgentSessionResponse,
} from "@research-assistant/core/agent";
import type { AgentSessionGateway } from "@research-assistant/data/agent";
import raw from "../../../../../../tests/fixtures/agent-rollback-events-v1.json";
import { AgentConversationController } from "./conversationController";
import { projectConversationMessages } from "./projectConversationMessages";

const events = raw.map(decodeAgentEvent);
const conversationId = events[0]!.conversationId;
const firstRollback = events.find(event => event.payload.type === "conversationRolledBack")!;
const id = (value: number): string => `00000000-0000-0000-0000-${value.toString(16).padStart(12, "0")}`;

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(yes => { resolve = yes; });
  return { promise, resolve };
}
function page(afterSeq: number, end: number, watermark: number): AgentHistoryResponse {
  return { schemaVersion: 1, conversationId, afterSeq, events: events.slice(afterSeq, end), watermark,
    nextAfterSeq: end < watermark ? end : null };
}

describe("production conversation rollback replay", () => {
  it("opens paginated rolled back history and publishes only the current transcript", async () => {
    const waiting = deferred<void>();
    const gateway: AgentSessionGateway = {
      create: vi.fn(),
      open: vi.fn(async (): Promise<AgentSessionResponse> => ({ schemaVersion: 1, conversationId, watermark: events.length })),
      readHistory: vi.fn(async request => page(request.afterSeq,
        Math.min(request.afterSeq + 5, events.length), events.length)),
      waitForHistory: vi.fn(() => { waiting.resolve(); return new Promise<AgentHistoryWaitResponse>(() => undefined); }),
    };
    const controller = new AgentConversationController(gateway);
    const seen: number[] = [];
    controller.subscribe(() => {
      const projection = controller.getSnapshot().projection;
      if (projection) seen.push(projection.seq);
    });
    try {
      await controller.open(conversationId);
      await waiting.promise;
      const snapshot = controller.getSnapshot();
      expect(snapshot.status).toBe("ready");
      expect(seen).toEqual([events.length]);
      expect(snapshot.projection?.turnOrder).toEqual([id(50)]);
      expect(snapshot.projection?.seq).toBe(events.length);
      const messages = projectConversationMessages(snapshot.projection);
      expect(messages).toHaveLength(1);
      expect(messages[0]).toMatchObject({ turnId: id(50), role: "user", content: "User input 50/1" });
      expect(raw).toHaveLength(events.length);
    } finally { controller.dispose(); }
  });

  it("applies a live rollback once and resumes after its increasing cursor", async () => {
    const before = firstRollback.seq - 1;
    const checkpoint = applyAgentEvents(createAgentConversationProjection(conversationId), events.slice(0, before));
    const waiting = deferred<void>(), changed = deferred<void>();
    const notice = deferred<AgentHistoryWaitResponse>();
    let watermark = before;
    const gateway: AgentSessionGateway = {
      create: vi.fn(),
      open: vi.fn(async (): Promise<AgentSessionResponse> => ({ schemaVersion: 1, conversationId, watermark })),
      readHistory: vi.fn(async request => page(request.afterSeq, watermark, watermark)),
      waitForHistory: vi.fn(() => { waiting.resolve(); return notice.promise; }),
    };
    const controller = new AgentConversationController(gateway, checkpoint);
    controller.subscribe(() => {
      if (controller.getSnapshot().projection?.seq === firstRollback.seq) changed.resolve();
    });
    try {
      await controller.open(conversationId);
      await waiting.promise;
      vi.mocked(gateway.waitForHistory).mockImplementation(() => new Promise(() => undefined));
      notice.resolve({ schemaVersion: 1, conversationId, afterSeq: before,
        watermark: firstRollback.seq, events: [firstRollback], resyncRequired: false });
      await changed.promise;
      const rolledBack = controller.getSnapshot().projection!;
      expect(rolledBack.turnOrder).toEqual([id(10), id(20)]);
      expect(rolledBack.seq).toBe(firstRollback.seq);
      expect(projectConversationMessages(rolledBack).map(message => message.turnId)).toEqual([id(10)]);
      expect(checkpoint.turnOrder).toEqual([id(10), id(20), id(30), id(40)]);
      controller.pause();
      watermark = events.length;
      const resumed = deferred<void>();
      vi.mocked(gateway.waitForHistory).mockImplementation(() => { resumed.resolve(); return new Promise(() => undefined); });
      await controller.resume();
      await resumed.promise;
      expect(gateway.readHistory).toHaveBeenLastCalledWith({ conversationId, afterSeq: firstRollback.seq, limit: 100 });
      expect(controller.getSnapshot().projection?.turnOrder).toEqual([id(50)]);
      expect(controller.getSnapshot().projection?.seq).toBe(events.length);
      expect(rolledBack.turnOrder).toEqual([id(10), id(20)]);
    } finally { controller.dispose(); }
  });
});
