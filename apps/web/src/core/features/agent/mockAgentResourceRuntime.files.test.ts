import {expect,it} from "vitest";
import {MockAgentResourceRuntime} from "./mockAgentResourceRuntime";
import {agentDraftInput,insertAgentReference} from "@research-assistant/core/agent";
it("missing and changed files remain ordinary text metadata without blocking a request",async()=>{
  const runtime=new MockAgentResourceRuntime();
  const draft=insertAgentReference({text:"读取",bindings:[]},{type:"mention",name:"本文阅读模板",target:{kind:"file",sourceId:"missing",resourceId:"missing"}});
  const result=await runtime.freeze(draft.text,agentDraftInput(draft));
  expect(result.userInput).toEqual(agentDraftInput(draft));expect(result.skillInstructions).toEqual([]);
  expect(result.skills.every(skill=>!skill.explicit)).toBe(true);
});
it("an unavailable skill tag is retained but never added to executable skills",async()=>{
  const runtime=new MockAgentResourceRuntime();
  const draft=insertAgentReference({text:"执行",bindings:[]},{type:"skill",name:"missing",identity:{environment_id:"application",source_id:"missing/SKILL.md",revision:"old"}});
  const result=await runtime.freeze(draft.text,agentDraftInput(draft));
  expect(result.userInput).toEqual(agentDraftInput(draft));expect(result.skillInstructions).toEqual([]);
  expect(result.skills.some(skill=>skill.descriptor.name==="missing")).toBe(false);
});
