import {expect, it, vi} from "vitest";
import type {AgentContextGateway} from "../../services/agentContextGateway";
import {nativeContextStartObservationFault} from "./nativeContextStartObservationFault";

it("loses one real start response and scopes the synthetic unavailable observation", async () => {
  const id = "11111111-1111-4111-8111-111111111111", other = "22222222-2222-4222-8222-222222222222";
  const request = {conversationId:id,requestId:id,contextSnapshotId:id,queue:null,resumeQueue:false};
  const result = {schemaVersion:1 as const,conversationId:id,requestId:id,turnId:id,runId:other,alreadyStarted:true,outcome:null};
  const start = vi.fn(async () => result);
  const read = vi.fn(async (intent:typeof request, expectedEpoch:string) => ({schemaVersion:1 as const,intent,expectedEpoch,
    status:{type:"started" as const,result:{...result,requestId:intent.requestId}}}));
  let enabled = true;
  const gateway = nativeContextStartObservationFault({guardedStart:{start,read}} as unknown as AgentContextGateway, () => enabled).guardedStart!;
  await expect(gateway.start(request,id)).rejects.toMatchObject({code:"AGENT_PREVIEW_SYNTHETIC_START_RESPONSE_LOST"});
  expect(start).toHaveBeenCalledOnce();
  const simulated = await gateway.read(request,id);
  expect(simulated.status).toMatchObject({type:"startedUnobserved",result:{turnId:id}});
  expect(simulated.status.type === "startedUnobserved" && simulated.status.result.runId).toBeUndefined();
  expect((await gateway.read({...request,requestId:other},id)).status.type).toBe("started");
  enabled = false;
  expect((await gateway.read(request,id)).status).toMatchObject({type:"started",result:{runId:other}});
});
