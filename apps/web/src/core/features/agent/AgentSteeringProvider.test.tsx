/** @vitest-environment happy-dom */
import type {AgentConversationProjection} from "@research-assistant/core/agent";
import {act,StrictMode} from "react";
import {createRoot} from "react-dom/client";
import {expect,it,vi} from "vitest";
import {AgentSteeringProvider,sendScopedHandoff,useAgentSteeringOwners} from "./AgentSteeringProvider";
import {AgentDraftProvider,useAgentDraftStore,useConversationDraft} from "./AgentDraftProvider";
import {useWritingSteering} from "../../../../../../plugins/writing_projects/src/features/writing/agent/useWritingSteering";
import {useNoteSteering} from "../notes/agent/useNoteSteering";
const mocks=vi.hoisted(()=>({writing:vi.fn(),notes:vi.fn(),paper:vi.fn(),context:vi.fn()}));
vi.mock("../../services/agentGateway",()=>({agentSteeringGateway:{steerWriting:mocks.writing},agentNoteSteeringGateway:{steer:mocks.notes},agentPaperSubmissionGateway:{steer:mocks.paper}}));
vi.mock("../../services/agentContextGateway",()=>({agentContextGateway:{steer:mocks.context}}));
(globalThis as typeof globalThis&{IS_REACT_ACT_ENVIRONMENT:boolean}).IS_REACT_ACT_ENVIRONMENT=true;

type Kind="manuscript"|"note"|"readingNote";
it("routes frozen handoff scopes to their original native steering gateway",async()=>{
  const request={conversationId:"conversation",turnId:"turn",inputId:"input",text:"message",userInput:[]};
  mocks.context.mockResolvedValue({});mocks.writing.mockResolvedValue({});mocks.paper.mockResolvedValue({});mocks.notes.mockResolvedValue({});
  await sendScopedHandoff(request,{kind:"context"});
  await sendScopedHandoff(request,{kind:"manuscript",manuscriptId:"manuscript"});
  await sendScopedHandoff(request,{kind:"paper",paperId:"paper"});
  await sendScopedHandoff(request,{kind:"note",resourceId:"note"});
  await sendScopedHandoff(request,{kind:"readingNote",resourceId:"reading"});
  expect(mocks.context).toHaveBeenCalledWith(request);
  expect(mocks.writing).toHaveBeenCalledWith({...request,manuscriptId:"manuscript"});
  expect(mocks.paper).toHaveBeenCalledWith({...request,paperId:"paper"});
  expect(mocks.notes).toHaveBeenCalledWith({...request,scope:{kind:"note",resourceId:"note"}});
  expect(mocks.notes).toHaveBeenCalledWith({...request,scope:{kind:"readingNote",resourceId:"reading"}});
});
async function harness(){
  let current!:ReturnType<typeof useWritingSteering>|ReturnType<typeof useNoteSteering>;
  let draft!:ReturnType<typeof useConversationDraft>;
  let owners!:ReturnType<typeof useAgentSteeringOwners>;
  function Host({kind,turn,projection}:{kind:Kind;turn:string|null;projection:AgentConversationProjection|null}){
    owners=useAgentSteeringOwners();
    draft=useConversationDraft(useAgentDraftStore(),{hostKind:kind,hostId:"same",conversationId:"conversation"});
    const host={key:JSON.stringify([kind,"same","conversation"]),conversationId:"conversation",captureAcknowledgment:draft[2]};
    // Both hooks are mounted but only the matching host owns a registry lease.
    const writing=useWritingSteering(kind==="manuscript"&&turn?{conversationId:"conversation",manuscriptId:"same",turnId:turn}:null,()=>undefined,projection,kind==="manuscript"?host:undefined);
    const notes=useNoteSteering(kind!=="manuscript"&&turn?{conversationId:"conversation",scope:{kind,resourceId:"same"},turnId:turn}:null,()=>undefined,projection,kind!=="manuscript"?host:undefined);
    current=kind==="manuscript"?writing:notes;
    return <p>{draft[0]}</p>;
  }
  const root=createRoot(document.createElement("div"));
  const render=async(kind:Kind|null,turn:string|null="turn",projection:AgentConversationProjection|null=null)=>{await act(async()=>root.render(<StrictMode><AgentSteeringProvider><AgentDraftProvider>
    {kind&&<Host key={kind} kind={kind} turn={turn} projection={projection}/>}
  </AgentDraftProvider></AgentSteeringProvider></StrictMode>));};
  await render("manuscript");
  return{render,get:()=>current,draft:()=>draft,owners:()=>owners!,close:async()=>{await act(async()=>root.unmount());}};
}

it("reattaches an unknown original request without an active turn and never clears a newer retyped draft",async()=>{
  let fail!:(reason:unknown)=>void;
  mocks.writing.mockReset().mockImplementationOnce(()=>new Promise((_resolve,reject)=>{fail=reject;})).mockResolvedValue({});
  const h=await harness();
  try{
    await act(async()=>h.draft()[1]("original"));let pending!:Promise<boolean>;
    await act(async()=>{pending=h.get().submit("original");});
    await h.render(null);fail(new Error("lost reply"));await act(async()=>pending);
    await h.render("manuscript",null);expect(h.get().uncertain).toBe(true);expect(h.get().state.request?.turnId).toBe("turn");
    await act(async()=>h.draft()[1]("changed"));await act(async()=>h.draft()[1]("original"));
    await act(async()=>h.get().retry());expect(mocks.writing.mock.calls[0]).toEqual(mocks.writing.mock.calls[1]);
    expect(h.draft()[0]).toBe("original");expect(h.get().unresolved).toBe(false);
  }finally{await h.close();}
});

it("acknowledges the exact original draft while its module is absent",async()=>{
  let finish!:()=>void;mocks.writing.mockReset().mockImplementation(()=>new Promise<void>(resolve=>{finish=resolve;}));
  const h=await harness();
  try{
    await act(async()=>h.draft()[1]("original"));let pending!:Promise<boolean>;
    await act(async()=>{pending=h.get().submit("original");});await h.render(null);
    await act(async()=>{finish();await pending;});await h.render("manuscript",null);
    expect(h.draft()[0]).toBe("");expect(h.get().state.awaitingHistory).toEqual([expect.objectContaining({text:"original"})]);expect(mocks.writing).toHaveBeenCalledOnce();
  }finally{await h.close();}
});

it("keeps typed note hosts and writing independent even with identical resource and conversation IDs",async()=>{
  mocks.writing.mockReset().mockRejectedValue(new Error("writing unknown"));
  mocks.notes.mockReset().mockRejectedValueOnce(new Error("note unknown")).mockRejectedValueOnce(new Error("reading unknown")).mockResolvedValue({});
  const h=await harness();
  try{
    await act(async()=>h.get().submit("writing"));await h.render("note");expect(h.get().unresolved).toBe(false);
    await act(async()=>h.draft()[1]("note"));await act(async()=>h.get().submit("note"));
    await h.render("readingNote");expect(h.get().unresolved).toBe(false);await act(async()=>h.get().submit("reading"));
    await h.render("note",null);expect(h.get().state.request?.text).toBe("note");await act(async()=>h.get().retry());expect(h.draft()[0]).toBe("");
    await h.render("readingNote",null);expect(h.get().state.request?.text).toBe("reading");
    await h.render("manuscript",null);expect(h.get().state.request?.text).toBe("writing");
    h.owners().writing.forgetConversation("conversation");await h.render(null);await h.render("manuscript");
    await act(async()=>h.get().submit("deleted"));expect(mocks.writing).toHaveBeenCalledOnce();
  }finally{await h.close();}
});

it("uses reloaded completed history to resolve a detached request without resending",async()=>{
  let fail!:(reason:unknown)=>void;mocks.writing.mockReset().mockImplementation(()=>new Promise((_resolve,reject)=>{fail=reject;}));
  const h=await harness();
  try{
    await act(async()=>h.draft()[1]("original"));let pending!:Promise<boolean>;
    await act(async()=>{pending=h.get().submit("original");});const inputId=h.get().state.request!.inputId;
    await h.render(null);
    const projection:AgentConversationProjection={conversationId:"conversation",seq:4,activeTurn:null,turnOrder:["turn"],turns:new Map([["turn",{
      contextSnapshotId:"context",model:{providerId:"fixture",modelId:"model",revision:null},phase:"preparing",outcome:"completed",
      itemOrder:[inputId],items:new Map([[inputId,{kind:{type:"userMessage"},text:"original",completed:true,interrupted:false}]]),
      attemptOrder:[],attempts:new Map(),inputTokens:null,outputTokens:null,
    }]])};
    await h.render("manuscript",null,projection);expect(h.get().unresolved).toBe(false);expect(h.draft()[0]).toBe("");
    await act(async()=>h.draft()[1]("new text"));await act(async()=>{fail(new Error("late failure"));await pending;});
    expect(h.get().unresolved).toBe(false);expect(h.draft()[0]).toBe("new text");expect(mocks.writing).toHaveBeenCalledOnce();
  }finally{await h.close();}
});
