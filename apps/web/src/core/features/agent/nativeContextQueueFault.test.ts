import {expect, it, vi} from "vitest";
import type {AgentContextGateway} from "../../services/agentContextGateway";
import type {NativeAgentPreview} from "../../runtime/nativeAgentPreview";
import {nativeContextQueueFault} from "./nativeContextQueueFault";

const id = (n:number) => `aaaaaaaa-aaaa-4aaa-8aaa-${String(n).padStart(12,"0")}`;
const ordinary = {conversationId:id(1),requestId:id(2),contextSnapshotId:id(3),queue:null,resumeQueue:false};
const queued = {...ordinary,queue:{inputId:id(4),expectedRevision:2}};
it("intercepts one queued start at native admission while preserving the real gateway and queue owner", async () => {
  const start = vi.fn(async () => "native"), read = vi.fn(), queue = {read:vi.fn(),edit:vi.fn()};
  const gateway = {guardedStart:{start,read},queue} as unknown as AgentContextGateway;
  const invoke = vi.fn(async () => ({schemaVersion:1,admitted:true})), preview = {invoke} as unknown as NativeAgentPreview;
  let armed = true; const wrapped = nativeContextQueueFault(gateway,preview,()=>{const value=armed;armed=false;return value;});
  expect(wrapped.queue).toBe(queue); expect(wrapped.guardedStart!.read).toBe(read);
  await expect(wrapped.guardedStart!.start(ordinary,id(5))).resolves.toBe("native"); expect(armed).toBe(true);
  await expect(wrapped.guardedStart!.start(queued,id(5))).rejects.toEqual({code:"AGENT_PREVIEW_SYNTHETIC_IO_START_ADMITTED"});
  expect(invoke).toHaveBeenCalledExactlyOnceWith("agent_preview_admit_context_queue",{request:queued,expectedEpoch:id(5)});
  await expect(wrapped.guardedStart!.start(queued,id(5))).resolves.toBe("native"); expect(start).toHaveBeenCalledTimes(2);
});
it("does not fake a lost receipt when native admission was not confirmed", async () => {
  const start = vi.fn(), gateway = {guardedStart:{start,read:vi.fn()}} as unknown as AgentContextGateway;
  const cause = {code:"DATA_SQL_RECOVERY_REQUIRED"}, invoke = vi.fn().mockRejectedValue(cause);
  const wrapped = nativeContextQueueFault(gateway,{invoke} as unknown as NativeAgentPreview,()=>true);
  await expect(wrapped.guardedStart!.start(queued,id(5))).rejects.toBe(cause); expect(start).not.toHaveBeenCalled();
});
