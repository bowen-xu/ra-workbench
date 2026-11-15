import {expect, it} from "vitest";
import {createAgentConversationProjection, type AgentTurnProjection} from "@research-assistant/core/agent";
import {projectConversationMessages} from "./projectConversationMessages";

function turn(text: string, finished = true): AgentTurnProjection {
  return {contextSnapshotId:"context", model:{providerId:"test",modelId:"test",revision:null},phase:"runningModel",outcome:finished?"completed":null,
    itemOrder:["answer"],items:new Map([["answer",{kind:{type:"assistantMessage"},text,completed:finished,interrupted:false}]]),
    attemptOrder:[],attempts:new Map(),inputTokens:null,outputTokens:null};
}
it("retains unchanged message identities when a different turn streams and removes rolled-back turns", () => {
  const old = turn("旧消息"), live = turn("流式",false);
  const first = {...createAgentConversationProjection("conversation"),activeTurn:"new",turnOrder:["old","new"],turns:new Map([["old",old],["new",live]])};
  const before = projectConversationMessages(first);
  const second = {...first,seq:2,turns:new Map(first.turns).set("new",turn("流式追加",false))};
  const after = projectConversationMessages(second);
  expect(after[0]).toBe(before[0]); expect(after[1]?.content).toBe("流式追加");
  expect(projectConversationMessages({...second,turnOrder:["new"]}).map(message=>message.turnId)).toEqual(["new"]);
});
it("does not reuse display data across conversation identities", () => {
  const source = {...createAgentConversationProjection("one"),turnOrder:["turn"],turns:new Map([["turn",turn("正文")]])};
  const first = projectConversationMessages(source);
  const other = projectConversationMessages({...source,conversationId:"two"});
  expect(first[0]?.conversationId).toBe("one"); expect(other[0]?.conversationId).toBe("two");
});
