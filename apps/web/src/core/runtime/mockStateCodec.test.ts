import {expect,it} from "vitest";
import {encodeMockState,decodeMockState} from "./mockStateCodec";
import {decodeAgentEvent} from "@research-assistant/core/agent";
it("stores completed message text once, removes old delta strings, and restores live cursors exactly",()=>{
  const conversationId=crypto.randomUUID(),turnId=crypto.randomUUID(),itemId=crypto.randomUUID(),active=crypto.randomUUID(),requestId=crypto.randomUUID();
  const payloads=[{type:"itemStarted",turnId,itemId,kind:{type:"assistantMessage"}},
    {type:"textDelta",turnId,itemId,text:"你好🙂"},{type:"textDelta",turnId,itemId,text:"\0tail"},
    {type:"itemCompleted",turnId,itemId},{type:"itemStarted",turnId,itemId:active,kind:{type:"assistantMessage"}},
    {type:"textDelta",turnId,itemId:active,text:"active text"}];
  const events=payloads.map((payload,i)=>decodeAgentEvent({schemaVersion:1,conversationId,seq:i+1,timestampMs:i,requestId,payload}));
  const state={events:new Map([[conversationId,events]]),finished:new Set(["done"])};
  const encoded=encodeMockState(state);
  expect(encoded.match(/"type":"textDelta"/g)).toHaveLength(1);
  expect(encoded).toContain("你好🙂\\u0000tail");
  expect(decodeMockState(encoded)).toEqual(state);
  expect(events[1]!.payload.type).toBe("textDelta");
  const old=JSON.stringify(events);
  expect(decodeMockState(old)).toEqual(events);
});
