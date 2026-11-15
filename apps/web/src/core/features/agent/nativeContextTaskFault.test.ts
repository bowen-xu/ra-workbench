import {expect, it, vi} from "vitest";
import type {AgentContextGateway} from "../../services/agentContextGateway";
import {nativeContextTaskFault} from "./nativeContextTaskFault";

it("changes only one completed task observation and returns to the original gateway when disabled", async () => {
  const id = "11111111-1111-4111-8111-111111111111", other = "22222222-2222-4222-8222-222222222222";
  const request = {conversationId:id,turnId:id,runId:id};
  let pending = true, enabled = true;
  const observe = vi.fn(async (identity: typeof request) => ({schemaVersion:1 as const,...identity,
    status: pending ? {type:"pending" as const} : {type:"finished" as const,completion:"completed" as const,diagnostic:null}}));
  const wrapped = nativeContextTaskFault({taskObservation:{observe}} as unknown as AgentContextGateway, () => enabled).taskObservation!;
  expect((await wrapped.observe(request)).status.type).toBe("pending");
  pending = false;
  expect((await wrapped.observe(request)).status).toMatchObject({type:"finished",completion:"recoveryRequired",
    diagnostic:{code:"PREVIEW_TASK_RECOVERY"}});
  expect((await wrapped.observe({...request,runId:other})).status).toMatchObject({type:"finished",completion:"completed"});
  enabled = false;
  expect((await wrapped.observe(request)).status).toMatchObject({type:"finished",completion:"completed"});
  expect(observe).toHaveBeenCalledTimes(4);
});
