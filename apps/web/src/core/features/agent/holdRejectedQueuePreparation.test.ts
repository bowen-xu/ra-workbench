import {afterEach, expect, it, vi} from "vitest";
import {WritingDiffDemoStorage} from "../../../app/previews/agent/WritingDiffDemoStorage";
import {WritingDiffDemoAgentStorage} from "../../../app/previews/agent/WritingDiffDemoAgentStorage";
import {createWritingDiffDemoRuntime} from "../../../app/previews/agent/writingDiffDemoRuntime";
import {holdRejectedQueuePreparation} from "./holdRejectedQueuePreparation";
const cleanups: (() => void)[] = [];
afterEach(() => {cleanups.splice(0).forEach(value => value()); vi.restoreAllMocks();});
async function fixture() {
  const scope = {manuscriptId: crypto.randomUUID(), conversationId: crypto.randomUUID()}, inputId = crypto.randomUUID();
  const files = new WritingDiffDemoStorage(scope.manuscriptId, []), agent = new WritingDiffDemoAgentStorage(files, undefined), runtime = createWritingDiffDemoRuntime(files, agent, scope.manuscriptId, scope.conversationId);
  cleanups.push(() => runtime.dispose()); const queue = await runtime.queue.gateway.edit({...scope, requestId: crypto.randomUUID(), expectedRevision: 0, edit: {type: "add", inputId, text: "original"}});
  const intent = {inputId, expectedRevision: queue.revision, submission: {conversationId: scope.conversationId, requestId: crypto.randomUUID(), text: "original", context: {manuscriptId: scope.manuscriptId, files: []}}};
  const lease = runtime.gateways.queue.registry.acquire(scope)!; return {scope, inputId, runtime, queue, intent, lease};
}
it("uses the existing queue owner to retry an unknown PauseInput with exactly the original edit identity", async () => {
  const f = await fixture(); f.runtime.queue.failNextEdit("afterCommit"); const edit = vi.spyOn(f.runtime.queue.gateway, "edit");
  await holdRejectedQueuePreparation(f.lease, f.intent); expect(edit).toHaveBeenCalledTimes(2); expect(edit.mock.calls[0]).toEqual(edit.mock.calls[1]);
  expect((await f.runtime.queue.gateway.read(f.scope)).inputs[0]).toMatchObject({inputId: f.inputId, text: "original", paused: true});
});
it("a changed input cannot be paused by an old preparation return action", async () => {
  const f = await fixture(); await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: f.queue.revision, edit: {type: "update", inputId: f.inputId, text: "new payload"}});
  const edit = vi.spyOn(f.runtime.queue.gateway, "edit"); await expect(holdRejectedQueuePreparation(f.lease, f.intent)).rejects.toThrow("QUEUE_PREPARE_INPUT_CHANGED"); expect(edit).not.toHaveBeenCalled();
});
it("a disposal during current-queue read cannot dispatch a later hold mutation", async () => {
  const f = await fixture(); let finish!: () => void, current = true; const read = f.runtime.queue.gateway.read;
  vi.spyOn(f.runtime.queue.gateway, "read").mockImplementationOnce(async scope => {const result = await read(scope); await new Promise<void>(resolve => {finish = resolve;}); return result;});
  const edit = vi.spyOn(f.runtime.queue.gateway, "edit"), work = holdRejectedQueuePreparation(f.lease, f.intent, () => current), result = work.catch(cause => cause);
  await vi.waitFor(() => expect(finish).toBeTypeOf("function")); current = false; finish(); expect(await result).toBeInstanceOf(Error); expect(edit).not.toHaveBeenCalled();
});
