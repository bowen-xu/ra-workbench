// @vitest-environment happy-dom
import { expect, it } from "vitest";
import { EMPTY_AGENT_CONTEXT, type AgentContextConfigurationSaveIntent, type AgentContextConfigurationSeedIntent, type AgentContextState } from "@research-assistant/core/agent";
import type { AgentContextConfigurationSaveGateway, AgentContextConfigurationSeedGateway } from "@research-assistant/data/agent";
import type { ConversationOperationLock } from "../conversationOperationLock";
import { ContextConfigurationStorageOutbox } from "./contextConfigurationOutbox";
import { ContextConfigurationOwner } from "./contextConfigurationOwner";
import { ContextConfigurationOwners } from "./contextConfigurationOwners";
import { ContextConfigurationSeedOwner } from "./contextConfigurationSeedOwner";
import { ContextConfigurationStartup } from "./contextConfigurationStartup";
import { contextConfigurationStartup } from "./contextConfigurationStartupFactory";
import { contextStorageKey } from "./contextStorage";
import { recoveryDraftKey } from "../../../services/recoveryDraftStore";

const conversation = "11111111-1111-4111-8111-111111111111", epoch = "22222222-2222-4222-8222-222222222222";
const legacy: AgentContextState = { ...EMPTY_AGENT_CONTEXT, persistent: [{ label: "Synthetic", resource: { kind: "paperMetadata", resourceId: "synthetic" } }] };
const trusted: AgentContextState = { ...EMPTY_AGENT_CONTEXT, persistent: [{ label: "Trusted", resource: { kind: "paperMetadata", resourceId: "trusted" } }] };

function fixture(seedInternal: boolean, seedCreates = false) {
  localStorage.clear();
  let sequence = 0, revision = 0, state: AgentContextState = EMPTY_AGENT_CONTEXT, failSeed = false, failSave = false;
  let environmentGate: Promise<void> | undefined, releaseEnvironment: (() => void) | undefined, environmentReads = 0;
  const seedCalls: AgentContextConfigurationSeedIntent[] = [], saveCalls: AgentContextConfigurationSaveIntent[] = [];
  const seedReceipts = new Set<string>(), saveReceipts = new Set<string>();
  const held = new Set<string>();
  const lock: ConversationOperationLock = { async run<T>(name: string, action: () => Promise<T>): Promise<T> {
    if (held.has(name)) throw Error("LOCK_BUSY"); held.add(name); try { return await action(); } finally { held.delete(name); }
  } };
  const newId = () => `00000000-0000-4000-8000-${String(++sequence).padStart(12, "0")}`;
  const saveProof = (intent: AgentContextConfigurationSaveIntent) => ({ schemaVersion: 1 as const, operationId: intent.operationId,
    conversationId: conversation, epoch, family: "agent.context.configuration.save" as const, sqlFence: "0", replayed: false, result: null });
  const seedProof = (intent: AgentContextConfigurationSeedIntent) => ({ schemaVersion: 1 as const, operationId: intent.operationId,
    conversationId: conversation, epoch, family: "agent.context.configuration.seed" as const, sqlFence: "0", replayed: false, result: null });
  const configuration: AgentContextConfigurationSaveGateway = {
    environment: async request => { environmentReads++; if (environmentGate) await environmentGate;
      return { schemaVersion: 1, ...request, epoch, sqlFence: "0", configuration: revision ? { revision, state } : null }; },
    status: async intent => saveReceipts.has(intent.operationId) ? { type: "committed", proof: saveProof(intent) }
      : { type: "noSqlReceipt", epoch, sqlFence: "0", closed: false },
    save: async intent => { saveCalls.push(structuredClone(intent)); if (failSave) { failSave = false; throw Error("save response lost"); }
      if (intent.expectedRevision !== revision) throw "confirmationMismatch";
      revision++; state = structuredClone(intent.state); saveReceipts.add(intent.operationId); return saveProof(intent); },
  };
  const gateway: AgentContextConfigurationSeedGateway = {
    status: async intent => seedReceipts.has(intent.operationId) ? { type: "committed", proof: seedProof(intent) }
      : { type: "noSqlReceipt", epoch, sqlFence: "0", closed: false },
    seed: async intent => { seedCalls.push(structuredClone(intent)); if (failSeed) { failSeed = false; throw Error("lost response"); }
      if (seedCreates && !revision) { state = structuredClone(trusted); revision = 1; }
      seedReceipts.add(intent.operationId); return seedProof(intent); },
  };
  const outbox = new ContextConfigurationStorageOutbox(localStorage, conversation, lock);
  const seed = new ContextConfigurationSeedOwner(conversation, outbox, configuration, gateway, newId);
  const owners = new ContextConfigurationOwners(id => ContextConfigurationOwner.open({ storage: localStorage, conversationId: id, gateway: configuration, newId, lock }));
  const startup = new ContextConfigurationStartup(conversation, legacy, seedInternal, outbox, seed, owners);
  return { startup, seedCalls, saveCalls, current: () => ({ revision, state }), failSeed: () => { failSeed = true; },
    failSave: () => { failSave = true; }, outbox, newId, configuration, gateway, lock,
    existingEmpty: () => { revision = 1; state = EMPTY_AGENT_CONTEXT; },
    environmentReads: () => environmentReads,
    holdEnvironment: () => { environmentGate = new Promise<void>(resolve => { releaseEnvironment = resolve; }); },
    releaseEnvironment: () => { const release = releaseEnvironment; environmentGate = undefined; release?.(); } };
}

it("imports old selections only when there is no existing configuration and no internal seed", async () => {
  const test = fixture(false);
  const result = await test.startup.open();
  expect(result.type).toBe("ready");
  if (result.type !== "ready") return;
  expect(test.seedCalls).toHaveLength(0); expect(test.saveCalls).toHaveLength(1);
  expect(test.current()).toEqual({ revision: 1, state: legacy });
  expect(result.lease.owner.getSnapshot().session.content).toEqual(legacy);
  result.lease.release();
});

it("shares scoped startup and imports the cache captured from that storage", async () => {
  const test = fixture(false);
  localStorage.setItem(contextStorageKey(conversation), JSON.stringify(legacy));
  const first = contextConfigurationStartup(localStorage, conversation, test.configuration, test.gateway, test.lock);
  const second = contextConfigurationStartup(localStorage, conversation.toUpperCase(), test.configuration, test.gateway, test.lock);
  expect(second).toBe(first);
  expect(contextConfigurationStartup(localStorage, conversation, test.configuration, { ...test.gateway }, test.lock)).not.toBe(first);
  const result = await first.open();
  expect(result.type).toBe("ready");
  if (result.type !== "ready") return;
  expect(test.seedCalls).toHaveLength(0); expect(test.current().state).toEqual(legacy);
  result.lease.release();
});

it("keeps trusted internal seed instead of importing the renderer cache", async () => {
  const test = fixture(true, true);
  const result = await test.startup.open();
  expect(result.type).toBe("ready");
  if (result.type !== "ready") return;
  expect(test.seedCalls).toHaveLength(1); expect(test.saveCalls).toHaveLength(0);
  expect(result.lease.owner.getSnapshot().session.observed.version).toBe(1);
  expect(result.lease.owner.getSnapshot().session.content).toEqual(trusted);
  result.lease.release();
});

it("preserves an existing empty configuration instead of restoring old cached selections", async () => {
  const test = fixture(true); test.existingEmpty();
  const result = await test.startup.open();
  expect(result.type).toBe("ready");
  if (result.type !== "ready") return;
  expect(test.seedCalls).toHaveLength(0); expect(test.saveCalls).toHaveLength(0);
  expect(result.lease.owner.getSnapshot().session.content).toEqual(EMPTY_AGENT_CONTEXT);
  result.lease.release();
});

it("exposes an unknown seed for explicit retry before configuration initialization", async () => {
  const test = fixture(true);
  test.failSeed(); await expect(test.startup.open()).rejects.toThrow("lost response");
  expect(await test.startup.open()).toEqual({ type: "seedPending" });
  expect(test.saveCalls).toHaveLength(0);
  await test.startup.retrySeed();
  expect(test.seedCalls[1]).toEqual(test.seedCalls[0]);
  const result = await test.startup.open();
  expect(result.type).toBe("ready");
  if (result.type !== "ready") return;
  expect(test.seedCalls).toHaveLength(2); expect(test.saveCalls).toHaveLength(1);
  result.lease.release();
});

it("shares seed and initial configuration across concurrent views", async () => {
  const test = fixture(true);
  const [first, second] = await Promise.all([test.startup.open(), test.startup.open()]);
  expect(first.type).toBe("ready"); expect(second.type).toBe("ready");
  if (first.type !== "ready" || second.type !== "ready") return;
  expect(first.lease.owner).toBe(second.lease.owner);
  expect(test.seedCalls).toHaveLength(1); expect(test.saveCalls).toHaveLength(1);
  first.lease.release(); second.lease.release();
});

it("does not replace an unresolved configuration save with an internal seed", async () => {
  const test = fixture(true);
  const intent = { operationId: test.newId(), conversationId: conversation, expectedEpoch: epoch, expectedRevision: 0, state: legacy };
  await test.outbox.run(() => test.outbox.write(intent));
  const result = await test.startup.open();
  expect(result.type).toBe("ready");
  if (result.type !== "ready") return;
  expect(result.lease.owner.getSnapshot().originalPending).toBe(true);
  expect(test.seedCalls).toHaveLength(0); expect(test.saveCalls).toHaveLength(0);
  result.lease.release();
});

it("does not seed or initialize after write authority is revoked while reading", async () => {
  const test = fixture(true); let writable = true;
  test.holdEnvironment();
  const opening = test.startup.open(() => writable);
  for (let n = 0; n < 100 && !test.environmentReads(); n++) await Promise.resolve();
  expect(test.environmentReads()).toBe(1);
  writable = false; test.releaseEnvironment();
  await expect(opening).rejects.toThrow("CONTEXT_CONFIGURATION_WRITE_REVOKED");
  expect(test.seedCalls).toHaveLength(0); expect(test.saveCalls).toHaveLength(0);
  expect(await test.outbox.run(() => test.outbox.kind())).toBe("empty");
});

it("returns the retained owner when a recovery checkpoint cannot be read", async () => {
  const test = fixture(false);
  const key = recoveryDraftKey({ scope: "agent-context-hbs", resourceId: conversation, slot: "configuration" });
  localStorage.setItem(key, "{broken");
  const result = await test.startup.open();
  expect(result.type).toBe("ready");
  if (result.type !== "ready") return;
  expect(result.lease.owner.getSnapshot().session.preservationError).toContain("MODEL_PRESERVATION_FAILED");
  expect(test.saveCalls).toHaveLength(0);
  await expect(result.lease.owner.repairLocal()).rejects.toThrow();
  expect(localStorage.getItem(key)).toBe("{broken");
  result.lease.release();
});

it("returns the original owner and cause when initialization has an unknown result", async () => {
  const test = fixture(false); test.failSave();
  const result = await test.startup.open();
  expect(result.type).toBe("ready");
  if (result.type !== "ready") return;
  expect(result.initializationError).toEqual(new Error("save response lost"));
  expect(result.lease.owner.getSnapshot().originalPending).toBe(true);
  await result.lease.owner.retryOriginal();
  expect(test.saveCalls[1]).toEqual(test.saveCalls[0]);
  result.lease.release();
});
