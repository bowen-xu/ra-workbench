// @vitest-environment happy-dom
import { beforeEach, expect, it } from "vitest";
import { EMPTY_AGENT_CONTEXT, type AgentContextConfigurationSeedIntent } from "@research-assistant/core/agent";
import { createContextConfigurationDocumentPort, type AgentContextConfigurationSaveGateway, type AgentContextConfigurationSeedGateway } from "@research-assistant/data/agent";
import { configurePreviewDatabase } from "../../../runtime/previewIndexedDatabase";
import { createIndexedPreviewStorage } from "../../../runtime/previewIndexedStorage";
import { createMemoryPreviewDatabase } from "../../../../app/runtime/testing/memoryPreviewDatabase";
import type { ConversationOperationLock } from "../conversationOperationLock";
import { ContextConfigurationStorageOutbox } from "./contextConfigurationOutbox";
import { ContextConfigurationSeedOwner } from "./contextConfigurationSeedOwner";
import { ContextConfigurationStartup } from "./contextConfigurationStartup";
import { ContextConfigurationOwners } from "./contextConfigurationOwners";

const conversation = "11111111-1111-4111-8111-111111111111", epoch = "22222222-2222-4222-8222-222222222222";
let database: ReturnType<typeof createMemoryPreviewDatabase>, lock: ConversationOperationLock;
beforeEach(() => {
  database = createMemoryPreviewDatabase(); configurePreviewDatabase(database.open);
  const held = new Set<string>();
  lock = { async run<T>(name: string, operation: () => Promise<T>): Promise<T> {
    if (held.has(name)) throw Error("LOCK_BUSY");
    held.add(name); try { return await operation(); } finally { held.delete(name); }
  } };
});
function fixture(storage: Storage) {
  let sequence = 0, lost = false, loseReceipt = false, existing = false, blocked = false;
  const calls: AgentContextConfigurationSeedIntent[] = [], receipts = new Set<string>();
  const proof = (intent: AgentContextConfigurationSeedIntent) => ({ schemaVersion: 1 as const, operationId: intent.operationId,
    conversationId: conversation, epoch, family: "agent.context.configuration.seed" as const, sqlFence: "0", replayed: false, result: null });
  const configuration: AgentContextConfigurationSaveGateway = {
    environment: async request => ({ schemaVersion: 1, ...request, epoch, sqlFence: "0", configuration: existing ? { revision: 1, state: EMPTY_AGENT_CONTEXT } : null }),
    save: async () => { throw Error("unexpected configuration save"); }, status: async () => { throw Error("unexpected configuration status"); },
  };
  const gateway: AgentContextConfigurationSeedGateway = {
    seed: async intent => { calls.push(structuredClone(intent)); if (lost) { lost = false; throw Error("lost response"); }
      receipts.add(intent.operationId); if (loseReceipt) { loseReceipt = false; throw Error("receipt lost"); } return proof(intent); },
    status: async intent => receipts.has(intent.operationId) ? { type: "committed", proof: proof(intent) }
      : blocked ? { type: "unconfirmed", epoch, sqlFence: "0", closed: false, blocked: true, reason: "owner_blocked_receipt_unconfirmed" }
        : { type: "noSqlReceipt", epoch, sqlFence: "0", closed: false },
  };
  const newId = () => `00000000-0000-4000-8000-${String(++sequence).padStart(12, "0")}`;
  const create = (value: Storage) => {
    const outbox = new ContextConfigurationStorageOutbox(value, conversation, lock);
    return { outbox, owner: new ContextConfigurationSeedOwner(conversation, outbox, configuration, gateway, newId) };
  };
  return { ...create(storage), create, calls, configuration, newId, lose: () => { lost = true; },
    loseReceipt: () => { loseReceipt = true; }, makeExisting: () => { existing = true; }, block: () => { blocked = true; } };
}

it("persists the original seed before sending and retries it after reopening", async () => {
  const storage = await createIndexedPreviewStorage("context-seed-retry:"), test = fixture(storage);
  test.lose(); await expect(test.owner.start()).rejects.toThrow("lost response");
  const reopened = await createIndexedPreviewStorage("context-seed-retry:"), next = test.create(reopened);
  expect(await next.owner.inspect()).toBe("unconfirmed");
  await expect(next.owner.start()).rejects.toThrow("CONTEXT_CONFIGURATION_SEED_ORIGINAL_PENDING");
  await next.owner.retry();
  expect(test.calls[1]).toEqual(test.calls[0]);
  expect(await next.owner.inspect()).toBe("empty");
});

it("does not send if the seed outbox is not durable and repairs the original request explicitly", async () => {
  const storage = await createIndexedPreviewStorage("context-seed-failure:"), test = fixture(storage);
  database.failNext(new DOMException("full", "QuotaExceededError"));
  await expect(test.owner.start()).rejects.toThrow();
  expect(test.calls).toHaveLength(0);
  await expect(test.owner.inspect()).rejects.toThrow("CONTEXT_CONFIGURATION_RECOVERY_LOCAL_UNCONFIRMED");
  await test.outbox.repair();
  await test.owner.retry();
  expect(test.calls).toHaveLength(1);
  expect(await test.owner.inspect()).toBe("empty");
});

it("repairs an unconfirmed seed outbox before the configuration owner can open", async () => {
  const storage = await createIndexedPreviewStorage("context-seed-startup-repair:"), test = fixture(storage);
  const owners = new ContextConfigurationOwners(async () => { throw Error("unexpected owner open"); });
  const startup = new ContextConfigurationStartup(conversation, EMPTY_AGENT_CONTEXT, true, test.outbox, test.owner, owners);
  database.failNext(new DOMException("full", "QuotaExceededError"));
  await expect(startup.open()).rejects.toThrow();
  expect(test.calls).toHaveLength(0);
  await startup.repairLocal();
  expect(await startup.open()).toEqual({ type: "seedPending" });
  await startup.retrySeed();
  expect(test.calls).toHaveLength(1);
  expect(await test.owner.inspect()).toBe("empty");
});

it("prevents a configuration save from replacing an unconfirmed seed", async () => {
  const storage = await createIndexedPreviewStorage("context-seed-shared:"), test = fixture(storage);
  test.lose(); await expect(test.owner.start()).rejects.toThrow("lost response");
  const port = createContextConfigurationDocumentPort(conversation, test.configuration, test.outbox, test.newId);
  await expect(port.read()).rejects.toThrow();
  expect(await test.owner.inspect()).toBe("unconfirmed");
  await test.owner.retry();
  expect((await port.read()).version).toBe(0);
});

it("does not seed when a configuration already exists", async () => {
  const storage = await createIndexedPreviewStorage("context-seed-existing:"), test = fixture(storage);
  test.makeExisting();
  expect(await test.owner.start()).toBe("existing");
  expect(test.calls).toHaveLength(0);
  expect(await test.owner.inspect()).toBe("empty");
});

it("confirms a lost receipt after reopening without executing the seed again", async () => {
  const storage = await createIndexedPreviewStorage("context-seed-receipt:"), test = fixture(storage);
  test.loseReceipt(); await expect(test.owner.start()).rejects.toThrow("receipt lost");
  const reopened = await createIndexedPreviewStorage("context-seed-receipt:"), next = test.create(reopened);
  expect(await next.owner.inspect()).toBe("committed");
  expect(test.calls).toHaveLength(1);
  expect(await next.owner.inspect()).toBe("empty");
});

it("does not replace an existing configuration save with a seed request", async () => {
  const storage = await createIndexedPreviewStorage("context-seed-reverse:"), test = fixture(storage);
  const intent = { operationId: test.newId(), conversationId: conversation, expectedEpoch: epoch,
    expectedRevision: 0, state: EMPTY_AGENT_CONTEXT };
  await test.outbox.run(() => test.outbox.write(intent));
  await expect(test.owner.start()).rejects.toThrow();
  expect(test.calls).toHaveLength(0);
  expect(await test.outbox.run(() => test.outbox.read())).toEqual(intent);
});

it("retains the original request and does not retry while the data owner is blocked", async () => {
  const storage = await createIndexedPreviewStorage("context-seed-blocked:"), test = fixture(storage);
  test.lose(); await expect(test.owner.start()).rejects.toThrow("lost response");
  test.block();
  await expect(test.owner.retry()).rejects.toThrow("CONTEXT_CONFIGURATION_SEED_RECOVERY_REQUIRED");
  expect(test.calls).toHaveLength(1);
  expect(await test.owner.inspect()).toBe("unconfirmed");
});

it("stops a seed after its original intent was frozen if write access is revoked", async () => {
  const storage = await createIndexedPreviewStorage("context-seed-revoked:"), test = fixture(storage);
  let writable = true;
  const write = test.outbox.writeSeed.bind(test.outbox);
  test.outbox.writeSeed = async intent => { await write(intent); if (intent) writable = false; };
  await expect(test.owner.start(() => writable)).rejects.toThrow("CONTEXT_CONFIGURATION_WRITE_REVOKED");
  expect(test.calls).toHaveLength(0);
  expect(await test.owner.inspect()).toBe("unconfirmed");
  await expect(test.owner.retry(() => writable)).rejects.toThrow("CONTEXT_CONFIGURATION_WRITE_REVOKED");
  expect(test.calls).toHaveLength(0);
  writable = true; await test.owner.retry(() => writable);
  expect(test.calls).toHaveLength(1);
});
