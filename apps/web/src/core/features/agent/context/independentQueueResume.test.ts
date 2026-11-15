import {expect, it, vi} from "vitest";
import {AgentInputQueueController} from "@research-assistant/data/agent";
import type {AgentContextQueueEditRequest, AgentQueueState} from "@research-assistant/core/agent";
import {independentQueueResume} from "./independentQueueResume";

const conversationId = "11111111-1111-4111-8111-111111111111", epoch = "22222222-2222-4222-8222-222222222222";
const requestId = "33333333-3333-4333-8333-333333333333";
const paused: AgentQueueState = {schemaVersion: 1, conversationId, revision: 8, paused: true, preparingInputId: null, inputs: []};
const command: AgentContextQueueEditRequest = {conversationId, requestId, expectedRevision: 8, expectedEpoch: epoch, edit: {type: "resume"}};
function fixture() {
  const read = vi.fn(async () => paused), edit = vi.fn(async (_request: AgentContextQueueEditRequest) => ({...paused, revision: 9, paused: false}));
  const owner = new AgentInputQueueController({read, edit}, {conversationId}, () => requestId), release = vi.fn();
  return {read, edit, owner, release, port: independentQueueResume(() => ({controller: owner, release}), read)};
}
it("saves the original epoch and command before sending through the shared owner", async () => {
  const f = fixture(); let release!: () => void;
  const save = vi.fn(async (value: AgentContextQueueEditRequest) => {expect(value).toEqual(command); await new Promise<void>(resolve => {release = resolve;});});
  const work = f.port.resume(conversationId, epoch, null, save, () => true);
  await vi.waitFor(() => expect(save).toHaveBeenCalledOnce()); expect(f.edit).not.toHaveBeenCalled();
  release(); expect(await work).toBe("acknowledged"); expect(f.edit).toHaveBeenCalledExactlyOnceWith(command); expect(f.release).toHaveBeenCalledOnce();
});
it("never drops an invalid empty epoch and sends an unguarded resume", async () => {
  const f = fixture(), save = vi.fn(async () => {});
  expect(await f.port.resume(conversationId, "", null, save, () => true)).toBe("unknown");
  expect(save).not.toHaveBeenCalled(); expect(f.edit).not.toHaveBeenCalled();
  expect(f.owner.getPendingRequest()).toEqual({...command, expectedEpoch: ""});
});
it("retains an uncertain original command across rejection and never substitutes a new identity", async () => {
  const f = fixture(); f.edit.mockRejectedValue({code: "conflict"});
  expect(await f.port.resume(conversationId, epoch, command, async () => {}, () => true)).toBe("unknown");
  expect(f.owner.getPendingRequest()).toEqual(command); expect(f.edit).toHaveBeenCalledExactlyOnceWith(command);
  await expect(f.port.resume(conversationId, crypto.randomUUID(), command, async () => {}, () => true)).rejects.toThrow("CONTEXT_QUEUE_RESUME_MISMATCH");
});
it("does not send when the owner becomes ineligible while saving", async () => {
  const f = fixture(); let eligible = true, release!: () => void;
  const save = vi.fn(async () => {await new Promise<void>(resolve => {release = resolve;});});
  const work = f.port.resume(conversationId, epoch, null, save, () => eligible);
  await vi.waitFor(() => expect(save).toHaveBeenCalledOnce()); eligible = false; release();
  expect(await work).toBe("unknown"); expect(f.edit).not.toHaveBeenCalled(); expect(f.owner.getPendingRequest()).toEqual(command);
});
it("does not replace another claim or a newer pending queue edit", async () => {
  const f = fixture(); f.read.mockResolvedValue({...paused, preparingInputId: requestId});
  await expect(f.port.resume(conversationId, epoch, command, async () => {}, () => true)).rejects.toThrow("CONTEXT_QUEUE_OTHER_CLAIM");
  expect(f.edit).not.toHaveBeenCalled();
  f.read.mockResolvedValue(paused); expect(f.owner.restorePending({...command, requestId: crypto.randomUUID()})).toBe(true);
  await expect(f.port.resume(conversationId, epoch, command, async () => {}, () => true)).rejects.toThrow("CONTEXT_QUEUE_OWNER_BUSY");
  expect(f.edit).not.toHaveBeenCalled();
});
it("rejects a queue read for a different conversation", async () => {
  const f = fixture(); f.read.mockResolvedValue({...paused, conversationId: crypto.randomUUID()});
  await expect(f.port.read(conversationId)).rejects.toThrow("CONTEXT_QUEUE_READ_MISMATCH");
});
it("does not manufacture a resume receipt for a queue that is already unpaused", async () => {
  const f = fixture(); f.read.mockResolvedValue({...paused, paused: false});
  expect(await f.port.resume(conversationId, epoch, null, async () => {}, () => true)).toBe("unknown");
  expect(f.edit).not.toHaveBeenCalled();
});
