import {beforeEach, expect, it, vi} from "vitest";
import type {AgentContextSelection} from "@research-assistant/core/agent";
const {invoke} = vi.hoisted(() => ({invoke:vi.fn()}));
vi.mock("../runtime/platformCore",()=>({invoke,isDesktopRuntime:()=>true}));
vi.mock("../runtime/appRuntime",()=>({getAppRuntime:()=>({mode:"desktop"}),runtimeAgentGateway:(_name:string,implementation:object)=>implementation}));
vi.mock("./agentGateway",()=>({agentSessionGateway:{},agentTurnGateway:{},agentTaskObservationGateway:{}}));
import {agentContextGateway} from "./agentContextGateway";
const id="00000000-0000-4000-8000-000000000001";
const candidate:AgentContextSelection={label:"Original",resource:{kind:"note",resourceId:id}};
beforeEach(()=>{invoke.mockReset();});
it.each(["note","paper","manuscriptFile","manuscript"] as const)("resolves a %s by exact identity without title or path search",async kind=>{
  const input:AgentContextSelection={label:"Ambiguous name",resource:kind==="manuscriptFile"?{kind,resourceId:id,path:"main.tex"}:kind==="manuscript"?{kind,resourceId:id,parts:[]}:{kind,resourceId:id}};
  const group:AgentContextSelection={label:"Actual title",resource:{kind:kind==="note"?"noteGroup":kind==="paper"?"paperGroup":"manuscript",resourceId:id,parts:[],newMemberAccess:"readWrite",groupAccess:"readWrite"}};
  invoke.mockResolvedValue(group);
  expect(await agentContextGateway.recommend!(input)).toEqual(group);
  expect(invoke).toHaveBeenCalledExactlyOnceWith("agent_recommend_context",{candidate:input});
});
it("preserves a reading-note leaf and rejects a wrong identity, kind or silent leaf fallback",async()=>{
  invoke.mockResolvedValue(candidate);expect(await agentContextGateway.recommend!(candidate)).toEqual(candidate);
  for(const resource of [{kind:"note",resourceId:"other"},{kind:"paperMetadata",resourceId:id}]){
    invoke.mockResolvedValue({label:"wrong",resource});
    await expect(agentContextGateway.recommend!(candidate)).rejects.toThrow("身份不匹配");
  }
  const paper:AgentContextSelection={label:"Paper",resource:{kind:"paper",resourceId:id}};
  invoke.mockResolvedValue(paper);await expect(agentContextGateway.recommend!(paper)).rejects.toThrow("身份不匹配");
});
it("does not dispatch after cancellation and ignores a late native response",async()=>{
  const aborted=new AbortController();aborted.abort();
  await expect(agentContextGateway.recommend!(candidate,aborted.signal)).rejects.toMatchObject({name:"AbortError"});
  expect(invoke).not.toHaveBeenCalled();
  let finish!:(value:AgentContextSelection)=>void;
  invoke.mockImplementation(()=>new Promise(resolve=>{finish=resolve;}));
  const controller=new AbortController(),pending=agentContextGateway.recommend!(candidate,controller.signal);
  controller.abort();finish(candidate);
  await expect(pending).rejects.toMatchObject({name:"AbortError"});
});
