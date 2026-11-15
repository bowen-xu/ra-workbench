import { expect, it } from "vitest";
import { applyAgentEvents, createAgentConversationProjection, decodeAgentEvent, type AgentEventPayload } from "@research-assistant/core/agent";
import { projectConversationMessages } from "./projectConversationMessages";

const conversationId="11111111-1111-4111-8111-111111111111",turnId="22222222-2222-4222-8222-222222222222";
const userId="33333333-3333-4333-8333-333333333333",assistantId="44444444-4444-4444-8444-444444444444";
const model={providerId:"fixture",modelId:"synthetic",revision:null};
function projection(){
  const entries:readonly [number,AgentEventPayload][]=[
    [1000,{type:"turnStarted",turnId,contextSnapshotId:userId,model}],
    [2100,{type:"itemStarted",turnId,itemId:userId,kind:{type:"userMessage"}}],
    [2200,{type:"textDelta",turnId,itemId:userId,text:"Original user message"}],
    [2300,{type:"itemCompleted",turnId,itemId:userId}],
    [7000,{type:"itemAccepted",turnId,itemId:assistantId,kind:{type:"assistantMessage"},text:"Original atomic answer",phase:"finalAnswer",source:null}],
    [8000,{type:"turnFinished",turnId,outcome:"completed"}],
  ];
  return applyAgentEvents(createAgentConversationProjection(conversationId),entries.map(([timestampMs,payload],index)=>decodeAgentEvent({
    schemaVersion:1,conversationId,requestId:turnId,seq:index+1,timestampMs,payload,
  })));
}
it("projects per-item event times without replacing them with turn boundaries",()=>{
  const messages=projectConversationMessages(projection());
  expect(messages.map(({createdAt})=>createdAt)).toEqual([new Date(2100).toISOString(),new Date(7000).toISOString()]);
  expect(messages.map(({turnStartedAtMs,turnFinishedAtMs})=>[turnStartedAtMs,turnFinishedAtMs])).toEqual([[1000,8000],[1000,8000]]);
  expect(messages[1]).toMatchObject({id:`${turnId}/${assistantId}`,phase:"finalAnswer",rawData:{createdAtMs:7000,accepted:true}});
});
it.each([undefined,NaN,Infinity,8_640_000_000_000_001])("keeps unknown or unrepresentable item time absent (%s)",timestamp=>{
  const original=projection(),turn=original.turns.get(turnId);if(!turn)throw new Error("Missing synthetic turn");
  const items=new Map([...turn.items].map(([id,item])=>{const next={...item};if(timestamp===undefined)delete next.createdAtMs;else next.createdAtMs=timestamp;return[id,next] as const;}));
  const messages=projectConversationMessages({...original,turns:new Map(original.turns).set(turnId,{...turn,items})});
  expect(messages).toHaveLength(2);expect(messages.every(message=>message.createdAt===undefined)).toBe(true);
});
it("accepts Unix zero as a known item time",()=>{
  const original=projection(),turn=original.turns.get(turnId);if(!turn)throw new Error("Missing synthetic turn");
  const items=new Map([...turn.items].map(([id,item])=>[id,{...item,createdAtMs:0}] as const));
  expect(projectConversationMessages({...original,turns:new Map(original.turns).set(turnId,{...turn,items})}).map(message=>message.createdAt))
    .toEqual(["1970-01-01T00:00:00.000Z","1970-01-01T00:00:00.000Z"]);
});
