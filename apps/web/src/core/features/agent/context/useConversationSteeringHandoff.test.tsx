// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import {createAgentConversationProjection,type AgentSteeringHandoffIntent} from "@research-assistant/core/agent";
import {AgentSteeringHandoffRegistry,type AgentSteeringHandoffGateway} from "@research-assistant/data/agent";
import type {AgentContextGateway} from "../../../services/agentContextGateway";
import {useConversationSteeringHandoff} from "./useConversationSteeringHandoff";

(globalThis as typeof globalThis & {IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;
it("never exposes the previous conversation's ready state during a scope switch",async()=>{
  const first="11111111-1111-4111-8111-111111111111",second="22222222-2222-4222-8222-222222222222",epoch="33333333-3333-4333-8333-333333333333";
  const read=vi.fn(async(id:string)=>id===first?{schemaVersion:1 as const,conversationId:id,epoch,items:[]}:new Promise<never>(()=>undefined));
  const gateway={steer:async()=>({}),steeringHandoff:{read,prepare:async()=>{throw Error("unexpected");},finish:async()=>{throw Error("unexpected");},release:async()=>{throw Error("unexpected");}}} as unknown as AgentContextGateway;
  function Harness({id}:{id:string}){const result=useConversationSteeringHandoff(id,gateway,undefined,true,createAgentConversationProjection(id),()=>undefined);return <p>{result.blocked?"blocked":"ready"}</p>;}
  const container=document.createElement("div"),root=createRoot(container);document.body.append(container);
  try{
    await act(async()=>root.render(<Harness id={first}/>));await vi.waitFor(()=>expect(container.textContent).toBe("ready"));
    await act(async()=>root.render(<Harness id={second}/>));expect(container.textContent).toBe("blocked");
    expect(read).toHaveBeenCalledWith(second,undefined);
  }finally{await act(async()=>root.unmount());container.remove();}
});

it("uses the scoped registry for a manuscript and never falls back to the context sender",async()=>{
  const id="11111111-1111-4111-8111-111111111111",turn="22222222-2222-4222-8222-222222222222",epoch="33333333-3333-4333-8333-333333333333",input="44444444-4444-4444-8444-444444444444",manuscript="55555555-5555-4555-8555-555555555555";
  const contextSend=vi.fn(),scopedSend=vi.fn(async()=>({}));
  const prepare=vi.fn(async(intent:AgentSteeringHandoffIntent)=>({schemaVersion:1 as const,operationId:intent.operationId,conversationId:id,family:"agent.steering.handoff.prepare" as const,epoch,sqlFence:"0",replayed:false,result:null}));
  const handoff={read:async()=>({schemaVersion:1 as const,conversationId:id,epoch,items:[]}),prepare,
    finish:async()=>{throw Error("unexpected");},release:async()=>{throw Error("unexpected");},cancelUnregistered:async()=>{throw Error("unexpected");},resolution:async()=>"unconfirmed" as const} satisfies AgentSteeringHandoffGateway;
  const gateway={steer:contextSend,steeringHandoff:handoff} as unknown as AgentContextGateway;
  const registry=new AgentSteeringHandoffRegistry(handoff,scopedSend,()=>crypto.randomUUID(),scope=>scope.kind==="manuscript");
  let hook!:ReturnType<typeof useConversationSteeringHandoff>;
  function Harness({useRegistry}:{useRegistry:boolean}){hook=useConversationSteeringHandoff(id,gateway,useRegistry?registry:undefined,true,createAgentConversationProjection(id),()=>undefined,{kind:"manuscript",manuscriptId:manuscript});return null;}
  const container=document.createElement("div"),root=createRoot(container);document.body.append(container);
  try{
    await act(async()=>root.render(<Harness useRegistry={false}/>));expect(hook.available).toBe(false);
    await act(async()=>root.render(<Harness useRegistry/>));await vi.waitFor(()=>expect(hook.state?.phase).toBe("ready"));
    await act(async()=>{await hook.submit({schemaVersion:1,conversationId:id,revision:0,paused:false,preparingInputId:null,inputs:[{inputId:input,text:"queued"}]},turn,input);});
    expect(prepare).toHaveBeenCalledWith(expect.objectContaining({scope:{kind:"manuscript",manuscriptId:manuscript}}));
    expect(scopedSend).toHaveBeenCalledWith(expect.objectContaining({conversationId:id,turnId:turn}),{kind:"manuscript",manuscriptId:manuscript});
    expect(contextSend).not.toHaveBeenCalled();
  }finally{await act(async()=>root.unmount());registry.dispose();container.remove();}
});

it("rejects a registry from a different data gateway even when conversation IDs match",async()=>{
  const id="11111111-1111-4111-8111-111111111111",epoch="33333333-3333-4333-8333-333333333333";
  const read=vi.fn(async()=>({schemaVersion:1 as const,conversationId:id,epoch,items:[]}));
  const handoff={read,prepare:async()=>{throw Error("unexpected");},finish:async()=>{throw Error("unexpected");},
    release:async()=>{throw Error("unexpected");},cancelUnregistered:async()=>{throw Error("unexpected");},resolution:async()=>"unconfirmed" as const} satisfies AgentSteeringHandoffGateway;
  const other={...handoff,read:vi.fn(handoff.read)};
  const registry=new AgentSteeringHandoffRegistry(other,async()=>({}),()=>crypto.randomUUID(),()=>true);
  const gateway={steer:vi.fn(),steeringHandoff:handoff} as unknown as AgentContextGateway;
  let hook!:ReturnType<typeof useConversationSteeringHandoff>;
  function Harness(){hook=useConversationSteeringHandoff(id,gateway,registry,true,createAgentConversationProjection(id),()=>undefined,{kind:"manuscript",manuscriptId:epoch});return null;}
  const container=document.createElement("div"),root=createRoot(container);
  try{
    await act(async()=>root.render(<Harness/>));
    expect(hook.available).toBe(false);expect(hook.state).toBeNull();expect(read).not.toHaveBeenCalled();expect(other.read).not.toHaveBeenCalled();
  }finally{await act(async()=>root.unmount());registry.dispose();}
});

it("does not reuse a prior queue synchronization when the owner changes for the same conversation",async()=>{
  const id="11111111-1111-4111-8111-111111111111",turn="22222222-2222-4222-8222-222222222222",epoch="33333333-3333-4333-8333-333333333333";
  function fixture(){let item:AgentSteeringHandoffIntent|null=null,confirmed=false;
    const proof=(intent:AgentSteeringHandoffIntent,family:import("@research-assistant/core/agent").AgentSteeringHandoffFamily)=>({schemaVersion:1 as const,operationId:intent.operationId,conversationId:id,family,epoch,sqlFence:"0",replayed:false,result:null});
    const handoff={read:async()=>({schemaVersion:1 as const,conversationId:id,epoch,items:item?[{intent:item,proof:proof(item,"agent.steering.handoff.prepare"),history:confirmed?"confirmed" as const:"unconfirmed" as const}]:[]}),
      prepare:async(intent:AgentSteeringHandoffIntent)=>{item=intent;return proof(intent,"agent.steering.handoff.prepare");},
      finish:async(intent:AgentSteeringHandoffIntent)=>{item=null;return proof(intent,"agent.steering.handoff.finish");},
      release:async()=>{throw Error("unexpected");},cancelUnregistered:async()=>{throw Error("unexpected");},resolution:async()=>"unconfirmed" as const} satisfies AgentSteeringHandoffGateway;
    return {handoff,gateway:{steer:async()=>({}),steeringHandoff:handoff} as unknown as AgentContextGateway,
      registry:new AgentSteeringHandoffRegistry(handoff,async()=>({}),()=>crypto.randomUUID(),()=>true),confirm:()=>{confirmed=true;}};}
  const first=fixture(),second=fixture(),firstReload=vi.fn(async()=>true);
  let release!:(value:boolean)=>void;
  const secondReload=vi.fn().mockResolvedValueOnce(true).mockImplementation(()=>new Promise<boolean>(resolve=>{release=resolve;}));
  let hook!:ReturnType<typeof useConversationSteeringHandoff>;
  function Harness({other}:{other:boolean}){const value=other?second:first;hook=useConversationSteeringHandoff(id,value.gateway,value.registry,true,createAgentConversationProjection(id),other?secondReload:firstReload);return null;}
  const container=document.createElement("div"),root=createRoot(container);document.body.append(container);
  const queue=(inputId:string)=>({schemaVersion:1 as const,conversationId:id,revision:0,paused:false,preparingInputId:null,inputs:[{inputId,text:"queued"}]});
  try{
    await act(async()=>root.render(<Harness other={false}/>));await vi.waitFor(()=>expect(hook.state?.phase).toBe("ready"));
    const firstInput=crypto.randomUUID();await act(async()=>{await hook.submit(queue(firstInput),turn,firstInput);});
    first.confirm();await act(async()=>hook.refresh());await vi.waitFor(()=>expect(hook.queueSyncPending).toBe(false));
    await act(async()=>root.render(<Harness other/>));await vi.waitFor(()=>expect(hook.state?.phase).toBe("ready"));
    const secondInput=crypto.randomUUID();await act(async()=>{await hook.submit(queue(secondInput),turn,secondInput);});
    second.confirm();await act(async()=>hook.refresh());
    await vi.waitFor(()=>expect(secondReload).toHaveBeenCalledTimes(2));
    expect(hook.queueSyncPending).toBe(true);expect(hook.canSubmit(turn)).toBe(false);
    await act(async()=>release(true));await vi.waitFor(()=>expect(hook.queueSyncPending).toBe(false));
  }finally{await act(async()=>root.unmount());first.registry.dispose();second.registry.dispose();container.remove();}
});

it("passes a manuscript instance predicate through the real hook and retains a prepared handoff for recovery",async()=>{
  const id="11111111-1111-4111-8111-111111111111",turn="22222222-2222-4222-8222-222222222222",epoch="33333333-3333-4333-8333-333333333333",input="44444444-4444-4444-8444-444444444444";
  let disabled=false,release!:()=>void;
  let registered:AgentSteeringHandoffIntent|null=null;
  const proof=(intent:AgentSteeringHandoffIntent)=>({schemaVersion:1 as const,operationId:intent.operationId,conversationId:id,family:"agent.steering.handoff.prepare" as const,epoch,sqlFence:"0",replayed:false,result:null});
  const prepare=vi.fn(async(intent:AgentSteeringHandoffIntent)=>{
    registered=intent;
    await new Promise<void>(resolve=>{release=resolve;});
    return proof(intent);
  });
  const handoff={read:async()=>({schemaVersion:1 as const,conversationId:id,epoch,items:registered?[{intent:registered,proof:proof(registered),history:"unconfirmed" as const}]:[]}),prepare,
    finish:async()=>{throw Error("unexpected");},release:async()=>{throw Error("unexpected");},cancelUnregistered:async()=>{throw Error("unexpected");},resolution:async()=>"unconfirmed" as const} satisfies AgentSteeringHandoffGateway;
  const send=vi.fn(async()=>({})),registry=new AgentSteeringHandoffRegistry(handoff,send,()=>crypto.randomUUID(),scope=>scope.kind==="manuscript");
  const gateway={steer:vi.fn(),steeringHandoff:handoff} as unknown as AgentContextGateway;
  const projection=createAgentConversationProjection(id),onSettled=()=>undefined,isDisabled=()=>disabled;
  let hook!:ReturnType<typeof useConversationSteeringHandoff>;
  function Harness(){hook=useConversationSteeringHandoff(id,gateway,registry,true,projection,onSettled,{kind:"manuscript",manuscriptId:epoch},0,isDisabled);return null;}
  const root=createRoot(document.createElement("div"));
  try{
    await act(async()=>root.render(<Harness/>));await vi.waitFor(()=>expect(hook.state?.phase).toBe("ready"));
    let work!:Promise<boolean>;await act(async()=>{work=hook.submit({schemaVersion:1,conversationId:id,revision:0,paused:false,preparingInputId:null,inputs:[{inputId:input,text:"keep"}]},turn,input);});
    expect(prepare).toHaveBeenCalledOnce();disabled=true;
    await act(async()=>{release();expect(await work).toBe(false);});
    expect(send).not.toHaveBeenCalled();expect(hook.state?.intent).toMatchObject({queueInputId:input});
    expect(hook.state?.acceptedOperationIds).toEqual([]);
  }finally{await act(async()=>root.unmount());registry.dispose();}
});
