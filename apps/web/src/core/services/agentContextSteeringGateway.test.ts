import {beforeEach, expect, it, vi} from "vitest";

const mocks = vi.hoisted(() => ({invoke:vi.fn(), preview:vi.fn(), mode:"desktop"}));
vi.mock("../runtime/platformCore", () => ({invoke:mocks.invoke,isDesktopRuntime:()=>true}));
vi.mock("../runtime/appRuntime", () => ({
  getAppRuntime:()=>mocks.mode === "preview" ? {mode:"preview",invoke:mocks.preview} : {mode:"desktop"},
  runtimeAgentGateway:(_name:string,implementation:object)=>implementation,
}));
vi.mock("./agentGateway", () => ({agentSessionGateway:{},agentTurnGateway:{},agentTaskObservationGateway:{}}));
import {agentContextGateway} from "./agentContextGateway";

const request = {conversationId:"11111111-1111-4111-8111-111111111111",turnId:"22222222-2222-4222-8222-222222222222",inputId:"33333333-3333-4333-8333-333333333333",text:"More detail"};
const receipt = {schemaVersion:1,conversationId:request.conversationId,turnId:request.turnId,inputId:request.inputId};
beforeEach(()=>{mocks.invoke.mockReset();mocks.preview.mockReset();mocks.mode="desktop";});

it.each(["desktop","preview"])("uses the same context steering contract in %s",async mode=>{
  mocks.mode=mode;
  const invoke=mode==="preview"?mocks.preview:mocks.invoke;
  invoke.mockResolvedValue(receipt);
  expect(await agentContextGateway.steer!(request)).toEqual(receipt);
  expect(invoke).toHaveBeenCalledExactlyOnceWith("agent_steer_context_turn",{request});
});

it("rejects a receipt for another input identity",async()=>{
  mocks.invoke.mockResolvedValue({...receipt,inputId:crypto.randomUUID()});
  await expect(agentContextGateway.steer!(request)).rejects.toThrow("插话回执身份不匹配");
});

it.each(["desktop","preview"])("uses the production handoff gateway in %s",async mode=>{
  mocks.mode=mode;
  const invoke=mode==="preview"?mocks.preview:mocks.invoke;
  const epoch="44444444-4444-4444-8444-444444444444";
  const page={schemaVersion:1,conversationId:request.conversationId,epoch,items:[]};
  invoke.mockResolvedValue(page);
  expect(await agentContextGateway.steeringHandoff!.read(request.conversationId,epoch)).toEqual(page);
  expect(invoke).toHaveBeenCalledExactlyOnceWith("agent_read_steering_handoffs",{request:{conversationId:request.conversationId,expectedEpoch:epoch}});
});

it.each(["desktop","preview"])("uses the same proven context save gateway in %s",async mode=>{
  mocks.mode=mode;
  const invoke=mode==="preview"?mocks.preview:mocks.invoke;
  const epoch="44444444-4444-4444-8444-444444444444";
  const intent={operationId:request.inputId,conversationId:request.conversationId,expectedEpoch:epoch,expectedRevision:0,
    state:{schemaVersion:1 as const,persistent:[],temporary:[]}};
  const proof={schemaVersion:1,operationId:intent.operationId,conversationId:intent.conversationId,epoch,family:"agent.context.configuration.save",sqlFence:"0",replayed:false,result:null};
  invoke.mockResolvedValue(proof);
  expect(await agentContextGateway.configurationSave!.save(intent)).toEqual(proof);
  expect(invoke).toHaveBeenCalledExactlyOnceWith("agent_save_context_configuration_proven",{request:intent});
});
