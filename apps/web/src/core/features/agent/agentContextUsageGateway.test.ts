import { expect, it } from "vitest";
import { decodeContextUsage } from "./agentContextUsageGateway";
const model = {providerId:"openai",modelId:"model",revision:null};
function response() { return {schemaVersion:1,conversationId:"conversation",status:"observed",currentTokens:4889,
  observation:{sample:{model,actualModel:model,inputTokens:4835,outputTokens:54,budget:{context_window_tokens:258000}}}}; }
it("uses the native sampling window measurement instead of cumulative turn usage", () => {
  expect(decodeContextUsage(response(),"conversation")).toEqual({inputTokens:4889,contextWindowTokens:258000});
});
it("displays the frozen effective window once, independently from the compaction trigger", () => {
  const value=response();
  for(const [window,expected] of [[272000,258400],[500000,475000]]) {
    expect(decodeContextUsage({...value,observation:{sample:{...value.observation.sample,budget:{context_window_tokens:window,effective_percent:95,configured_trigger_tokens:200000}}}},"conversation"))
      .toEqual({inputTokens:4889,contextWindowTokens:expected});
  }
  expect(decodeContextUsage({...value,observation:{sample:{...value.observation.sample,budget:{context_window_tokens:500000,effective_percent:101}}}},"conversation"))
    .toEqual({inputTokens:4889});
});
it("labels a stale observation and does not manufacture a window size", () => {
  const value = {...response(),status:"stale",currentTokens:null};
  expect(decodeContextUsage(value,"conversation")).toEqual({inputTokens:4889,contextWindowTokens:258000,measurement:"stale"});
  expect(decodeContextUsage({...value,observation:{sample:{model,actualModel:model,inputTokens:4835,outputTokens:54}}},"conversation"))
    .toEqual({inputTokens:4889,measurement:"stale"});
  expect(decodeContextUsage({...value,observation:null},"conversation")).toBeUndefined();
});
it("rejects another conversation and never treats a transitioned model as measured usage", () => {
  expect(() => decodeContextUsage(response(),"other")).toThrow();
  const value = response();
  expect(decodeContextUsage({...value,currentTokens:null,observation:{sample:{...value.observation.sample,actualModel:{...model,modelId:"other"}}}},"conversation")).toBeUndefined();
});
