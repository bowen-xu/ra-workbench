// @vitest-environment happy-dom
import { expect, it } from "vitest";
import { EMPTY_AGENT_CONTEXT, type AgentContextConfigurationSaveIntent, type AgentContextState } from "@research-assistant/core/agent";
import type { AgentContextConfigurationSaveGateway } from "@research-assistant/data/agent";
import type { ConversationOperationLock } from "../conversationOperationLock";
import { ContextConfigurationOwner } from "./contextConfigurationOwner";
import { ContextConfigurationOwners, contextConfigurationOwners } from "./contextConfigurationOwners";

const conversation = "11111111-1111-4111-8111-111111111111";
const epoch = "22222222-2222-4222-8222-222222222222";
const item = { resource: { kind: "paperMetadata" as const, resourceId: "synthetic" }, label: "Synthetic" };
const first: AgentContextState = { ...EMPTY_AGENT_CONTEXT, persistent: [item] };
const later: AgentContextState = { ...first, temporary: [{ ...item, resource: { ...item.resource, resourceId: "synthetic-later" } }] };

function fixture(missing = false) {
  localStorage.clear();
  const held = new Set<string>();
  const lock: ConversationOperationLock = { async run<T>(name: string, task: () => Promise<T>): Promise<T> {
    if (held.has(name)) throw new Error("LOCK_BUSY");
    held.add(name); try { return await task(); } finally { held.delete(name); }
  } };
  let state: AgentContextState = EMPTY_AGENT_CONTEXT, revision = missing ? 0 : 1, sequence = 0;
  let fail = false, loseReceipt = false, failRead = false, release: (() => void) | undefined;
  const calls: AgentContextConfigurationSaveIntent[] = [];
  const receipts = new Set<string>();
  const proof = (intent: AgentContextConfigurationSaveIntent) => ({ schemaVersion: 1 as const, operationId: intent.operationId,
    conversationId: conversation, epoch, family: "agent.context.configuration.save" as const, sqlFence: "0", replayed: false, result: null });
  const gateway: AgentContextConfigurationSaveGateway = {
    environment: async request => { if (failRead) { failRead = false; throw Error("environment unavailable"); }
      return { schemaVersion: 1, ...request, epoch, sqlFence: "0", configuration: revision ? { revision, state } : null }; },
    status: async intent => receipts.has(intent.operationId) ? { type: "committed", proof: proof(intent) }
      : { type: "noSqlReceipt", epoch, sqlFence: "0", closed: false },
    save: async intent => {
      calls.push(structuredClone(intent));
      if (fail) { fail = false; throw new Error("transport lost"); }
      if (release) await new Promise<void>(resolve => { release = resolve; });
      if (receipts.has(intent.operationId)) return proof(intent);
      if (intent.expectedRevision !== revision) throw "confirmationMismatch";
      state = structuredClone(intent.state); revision += 1; receipts.add(intent.operationId);
      if (loseReceipt) { loseReceipt = false; throw new Error("receipt lost"); }
      return proof(intent);
    },
  };
  const options = { storage: localStorage, conversationId: conversation, gateway, lock,
    newId: () => `00000000-0000-4000-8000-${String(++sequence).padStart(12, "0")}` };
  return { options, calls, current: () => state, fail: () => { fail = true; }, loseReceipt: () => { loseReceipt = true; },
    failRead: () => { failRead = true; },
    external: (next: AgentContextState) => { state = structuredClone(next); revision += 1; },
    hold: () => { release = () => undefined; }, release: () => { const resolve = release; release = undefined; resolve?.(); } };
}

it("retains an owner during a save and preserves later input before its receipt", async () => {
  const test = fixture();
  const owner = await ContextConfigurationOwner.open(test.options);
  await owner.edit(first); test.hold();
  const saving = owner.save();
  for (let i = 0; i < 100 && !test.calls.length; i++) await Promise.resolve();
  expect(test.calls).toHaveLength(1);
  expect(owner.getSnapshot().phase).toBe("retained");
  expect(() => owner.dispose()).toThrow("CONTEXT_CONFIGURATION_OWNER_RETAINED");
  await owner.edit(later);
  test.release();
  await expect(saving).rejects.toThrow("CONTEXT_CONFIGURATION_SAVE_UNCONFIRMED");
  expect(owner.getSnapshot().session.content).toEqual(later);
  await owner.save();
  expect(test.current()).toEqual(later);
  expect(owner.getSnapshot().phase).toBe("idle");
  owner.dispose();
});

it("reconciles an external update without replacing later local input", async () => {
  const test = fixture(), owner = await ContextConfigurationOwner.open(test.options);
  await owner.edit(first);
  const external: AgentContextState = { ...EMPTY_AGENT_CONTEXT, temporary: later.temporary };
  test.external(external);
  await owner.refresh();
  expect(owner.getSnapshot().session.content).toEqual(later);
  expect(owner.getSnapshot().session.conflicts).toHaveLength(0);
  await owner.save();
  expect(test.current()).toEqual(later);
  owner.dispose();
});

it.each(["incoming", "keep"] as const)("resolves a concurrent ordinary configuration conflict through HBS: %s", async action => {
  const test = fixture(), owner = await ContextConfigurationOwner.open(test.options);
  await owner.edit(first);
  const remote: AgentContextState = { ...EMPTY_AGENT_CONTEXT, persistent: [{ ...item, resource: { ...item.resource, resourceId: "remote" } }] };
  test.external(remote); await owner.refresh();
  const conflict = owner.getSnapshot().session.conflicts[0]; expect(conflict).toBeDefined();
  await owner.resolveConflict(conflict!.id, action);
  expect(owner.getSnapshot().session.conflicts).toHaveLength(0);
  await owner.save();
  const expected = action === "incoming" ? remote : first;
  expect(owner.getSnapshot().session.content).toEqual(expected);
  expect(test.current()).toEqual(expected);
  owner.dispose();
});

it("reports an authority refresh failure and retains local input", async () => {
  const test = fixture(), owner = await ContextConfigurationOwner.open(test.options);
  await owner.edit(first); test.failRead();
  await expect(owner.refresh()).rejects.toThrow("environment unavailable");
  expect(owner.getSnapshot().session.content).toEqual(first);
  expect(owner.getSnapshot().phase).toBe("retained");
  await owner.save(); owner.dispose();
});

it("does not refresh past an unknown in-flight save before its original retry", async () => {
  const test = fixture(), owner = await ContextConfigurationOwner.open(test.options);
  await owner.edit(first); test.fail();
  await expect(owner.save()).rejects.toThrow("CONTEXT_CONFIGURATION_SAVE_UNCONFIRMED");
  await expect(owner.refresh()).rejects.toThrow("MODEL_RESULT_UNKNOWN");
  expect(owner.getSnapshot().session.content).toEqual(first);
  await owner.retrySave();
  expect(test.calls[1]).toEqual(test.calls[0]);
  owner.dispose();
});

it("keeps a started save but revokes a queued later save until write access returns", async () => {
  const test = fixture(), owner = await ContextConfigurationOwner.open(test.options);
  await owner.edit(first); test.hold();
  const saving = owner.save();
  const rejected = expect(saving).rejects.toThrow("CONTEXT_CONFIGURATION_SAVE_UNCONFIRMED");
  for (let i = 0; i < 100 && !test.calls.length; i++) await Promise.resolve();
  expect(test.calls).toHaveLength(1);
  await owner.edit(later);
  await expect(owner.save()).rejects.toThrow("CONTEXT_CONFIGURATION_SAVE_UNCONFIRMED");
  owner.setWritable(false);
  test.release(); await rejected;
  expect(test.calls).toHaveLength(1);
  expect(test.current()).toEqual(first);
  expect(owner.getSnapshot().session.content).toEqual(later);
  await expect(owner.retrySave()).rejects.toThrow("AGENT_CONTEXT_READ_ONLY");
  owner.setWritable(true);
  await owner.retrySave();
  expect(test.calls).toHaveLength(2);
  expect(test.current()).toEqual(later);
  owner.dispose();
});

it("blocks edits and unknown save retries while readonly without changing the original identity", async () => {
  const test = fixture(), owner = await ContextConfigurationOwner.open(test.options);
  await owner.edit(first); test.fail();
  await expect(owner.save()).rejects.toThrow("CONTEXT_CONFIGURATION_SAVE_UNCONFIRMED");
  owner.setWritable(false);
  await expect(owner.edit(later)).rejects.toThrow("AGENT_CONTEXT_READ_ONLY");
  await expect(owner.retrySave()).rejects.toThrow("AGENT_CONTEXT_READ_ONLY");
  expect(test.calls).toHaveLength(1);
  owner.setWritable(true); await owner.retrySave();
  expect(test.calls[1]).toEqual(test.calls[0]);
  owner.dispose();
});

it.each([EMPTY_AGENT_CONTEXT, first])("creates a missing row using the proven save even when the initial value is empty", async initial => {
  const test = fixture(true);
  const owner = await ContextConfigurationOwner.open(test.options);
  expect(await owner.initialize(initial)).toBe("created");
  expect(test.calls).toHaveLength(1);
  expect(test.calls[0]?.expectedRevision).toBe(0);
  expect(test.current()).toEqual(initial);
  expect(owner.getSnapshot().session.observed.version).toBe(1);
  expect(await owner.initialize(EMPTY_AGENT_CONTEXT)).toBe("existing");
  expect(test.calls).toHaveLength(1);
  owner.dispose();
});

it("keeps edits made while the missing row is being initialized", async () => {
  const test = fixture(true), owner = await ContextConfigurationOwner.open(test.options);
  test.hold(); const initializing = owner.initialize(first);
  for (let i = 0; i < 100 && !test.calls.length; i++) await Promise.resolve();
  expect(test.calls).toHaveLength(1);
  await owner.edit(later);
  test.release();
  expect(await initializing).toBe("created");
  expect(owner.getSnapshot().session.content).toEqual(later);
  await owner.save(); expect(test.current()).toEqual(later);
  owner.dispose();
});

it("retains the frozen initial value after an unknown initialization result", async () => {
  const test = fixture(true), owner = await ContextConfigurationOwner.open(test.options);
  test.fail();
  await expect(owner.initialize(first)).rejects.toThrow("transport lost");
  expect(owner.getSnapshot().originalPending).toBe(true);
  await owner.edit(later);
  await expect(owner.initialize(later)).rejects.toThrow("CONTEXT_CONFIGURATION_INITIALIZATION_BLOCKED");
  await owner.retryOriginal();
  expect(test.calls[1]).toEqual(test.calls[0]);
  expect(owner.getSnapshot().session.content).toEqual(later);
  await owner.save(); expect(test.current()).toEqual(later);
  owner.dispose();
});

it("revokes initialization before sending and preserves its frozen request", async () => {
  const test = fixture(true), owner = await ContextConfigurationOwner.open(test.options);
  let checks = 0;
  await expect(owner.initialize(first, () => ++checks < 3)).rejects.toThrow("CONTEXT_CONFIGURATION_WRITE_REVOKED");
  expect(owner.getSnapshot().originalPending).toBe(true);
  expect(test.calls).toHaveLength(0);
  await expect(owner.retryOriginal(() => false)).rejects.toThrow("CONTEXT_CONFIGURATION_WRITE_REVOKED");
  expect(test.calls).toHaveLength(0);
  await owner.retryOriginal();
  expect(test.current()).toEqual(first); expect(test.calls).toHaveLength(1);
  owner.dispose();
});

it("does not leave a pending barrier when initialization is revoked before freezing", async () => {
  const test = fixture(true), owner = await ContextConfigurationOwner.open(test.options);
  await expect(owner.initialize(first, () => false)).rejects.toThrow("CONTEXT_CONFIGURATION_WRITE_REVOKED");
  expect(owner.getSnapshot().originalPending).toBe(false);
  expect(await owner.initialize(first)).toBe("created");
  owner.dispose();
});

it("recognizes a committed initialization by its receipt after the response is lost", async () => {
  const test = fixture(true), owner = await ContextConfigurationOwner.open(test.options);
  test.loseReceipt();
  expect(await owner.initialize(first)).toBe("created");
  expect(owner.getSnapshot().originalPending).toBe(false);
  expect(owner.getSnapshot().session.observed.version).toBe(1);
  owner.dispose();
});

it("does not replace a configuration created by another owner after the initial read", async () => {
  const test = fixture(true), owner = await ContextConfigurationOwner.open(test.options);
  test.external(later);
  expect(await owner.initialize(first)).toBe("existing");
  expect(test.current()).toEqual(later);
  expect(owner.getSnapshot().session.observed.saved).toEqual(later);
  expect(owner.getSnapshot().originalPending).toBe(false);
  owner.dispose();
});

it("opens after a committed receipt is lost and retains later edits", async () => {
  const test = fixture();
  const original = await ContextConfigurationOwner.open(test.options);
  await original.edit(first); test.loseReceipt();
  await expect(original.save()).rejects.toThrow("CONTEXT_CONFIGURATION_SAVE_UNCONFIRMED");
  await original.edit(later);
  const reopened = await ContextConfigurationOwner.open(test.options);
  expect(reopened.getSnapshot().session.content).toEqual(later);
  expect(reopened.getSnapshot().originalPending).toBe(false);
});

it("recovers an unknown original request without replacing it with later input", async () => {
  const test = fixture();
  const original = await ContextConfigurationOwner.open(test.options);
  await original.edit(first); test.fail();
  await expect(original.save()).rejects.toThrow("CONTEXT_CONFIGURATION_SAVE_UNCONFIRMED");
  expect(original.getSnapshot().phase).toBe("retained");
  // A new host simulates reopening after the original process disappeared.
  const reopened = await ContextConfigurationOwner.open(test.options);
  expect(reopened.getSnapshot().originalPending).toBe(true);
  await reopened.edit(later);
  await expect(reopened.save()).rejects.toThrow("CONTEXT_CONFIGURATION_ORIGINAL_PENDING");
  await reopened.retryOriginal();
  expect(test.calls[1]).toEqual(test.calls[0]);
  expect(test.current()).toEqual(later);
  expect(reopened.getSnapshot().session.content).toEqual(later);
  await reopened.save();
  expect(test.current()).toEqual(later);
  reopened.dispose();
});

it("shares concurrent opens and retains the owner across views while a save is in flight", async () => {
  const test = fixture();
  let opens = 0;
  const registry = new ContextConfigurationOwners(async () => { opens += 1; return ContextConfigurationOwner.open(test.options); });
  const [firstView, secondView] = await Promise.all([registry.acquire(conversation), registry.acquire(conversation.toUpperCase())]);
  expect(opens).toBe(1);
  expect(firstView.owner).toBe(secondView.owner);
  await firstView.owner.edit(first); test.hold();
  const saving = firstView.owner.save();
  for (let i = 0; i < 100 && !test.calls.length; i++) await Promise.resolve();
  expect(test.calls).toHaveLength(1);
  firstView.release(); firstView.release(); secondView.release();
  const reopened = await registry.acquire(conversation);
  expect(reopened.owner).toBe(firstView.owner);
  test.release(); await saving;
  reopened.release();
  for (let i = 0; i < 100 && reopened.owner.getSnapshot().phase !== "idle"; i++) await Promise.resolve();
  const next = await registry.acquire(conversation);
  expect(next.owner).not.toBe(reopened.owner);
  expect(opens).toBe(2);
  next.release();
});

it("does not cache a failed open and isolates registries by storage and gateway", async () => {
  const test = fixture(); let attempts = 0;
  const registry = new ContextConfigurationOwners(async () => {
    if (++attempts === 1) throw new Error("open failed");
    return ContextConfigurationOwner.open(test.options);
  });
  await expect(registry.acquire(conversation)).rejects.toThrow("open failed");
  const recovered = await registry.acquire(conversation);
  expect(attempts).toBe(2); recovered.release();
  expect(contextConfigurationOwners(test.options.storage, test.options.gateway)).toBe(contextConfigurationOwners(test.options.storage, test.options.gateway));
  expect(contextConfigurationOwners(test.options.storage, { ...test.options.gateway })).not.toBe(contextConfigurationOwners(test.options.storage, test.options.gateway));
});
