import {createContext,useContext,useEffect,useState,type ReactNode} from "react";
import type {AgentNoteSteeringScope,AgentPaperSteerRequest,AgentQueueEditScope,AgentSteeringHandoffIntent,AgentWritingSteerRequest} from "@research-assistant/core/agent";
import {AgentSteeringHandoffRegistry,AgentSteeringRegistry} from "@research-assistant/data/agent";
import {agentNoteSteeringGateway,agentPaperSubmissionGateway,agentSteeringGateway} from "../../services/agentGateway";
import {agentContextGateway} from "../../services/agentContextGateway";
import {PendingSteeringOrderStore} from "./PendingSteeringOrderStore";
import {captureQueueOwnerIntent} from "./captureQueueOwnerIntent";
import type {AgentSteeringOwners} from "@research-assistant/workbench-sdk/agent-submission";

type WritingScope=Omit<AgentWritingSteerRequest,"inputId"|"text">;
type HandoffSend={conversationId:string;turnId:string;inputId:string;text:string;userInput:AgentSteeringHandoffIntent["userInput"]};
export function sendScopedHandoff(request:HandoffSend,scope:AgentQueueEditScope):Promise<unknown>{
  switch(scope.kind){
    case "context": return agentContextGateway.steer!(request);
    case "manuscript": return agentSteeringGateway.steerWriting({...request,manuscriptId:scope.manuscriptId});
    case "paper": return agentPaperSubmissionGateway.steer({...request,paperId:scope.paperId});
    case "note": case "readingNote": return agentNoteSteeringGateway.steer({...request,scope});
  }
}
function createOwners(){
  return {
    pendingOrder:new PendingSteeringOrderStore(),
    conversations:new AgentSteeringRegistry<{conversationId:string;turnId:string}>(request=>agentContextGateway.steer!(request),()=>crypto.randomUUID(),captureQueueOwnerIntent),
    handoffs:new AgentSteeringHandoffRegistry({
      read:(id,epoch)=>agentContextGateway.steeringHandoff!.read(id,epoch),
      prepare:intent=>agentContextGateway.steeringHandoff!.prepare(intent),
      resolution:(intent,epoch)=>agentContextGateway.steeringHandoff!.resolution(intent,epoch),
      finish:(intent,epoch)=>agentContextGateway.steeringHandoff!.finish(intent,epoch),
      release:(intent,epoch)=>agentContextGateway.steeringHandoff!.release(intent,epoch),
      cancelUnregistered:(intent,epoch)=>agentContextGateway.steeringHandoff!.cancelUnregistered(intent,epoch),
    },sendScopedHandoff,()=>crypto.randomUUID(),()=>true,agentContextGateway.steeringHandoff,captureQueueOwnerIntent),
    writing:new AgentSteeringRegistry<WritingScope>(request=>agentSteeringGateway.steerWriting(request),()=>crypto.randomUUID(),captureQueueOwnerIntent),
    notes:new AgentSteeringRegistry<AgentNoteSteeringScope>(request=>agentNoteSteeringGateway.steer(request),()=>crypto.randomUUID(),captureQueueOwnerIntent),
    papers:new AgentSteeringRegistry<Omit<AgentPaperSteerRequest,"inputId"|"text">>(request=>agentPaperSubmissionGateway.steer(request),()=>crypto.randomUUID(),captureQueueOwnerIntent),
  };
}
const Context=createContext<ReturnType<typeof createOwners>|null>(null);

export function AgentSteeringProvider({children}:{readonly children:ReactNode}):React.JSX.Element{
  const [owners]=useState(createOwners);
  useEffect(()=>{const writing=owners.writing.retain(),notes=owners.notes.retain(),papers=owners.papers.retain(),conversations=owners.conversations.retain(),handoffs=owners.handoffs.retain();return()=>{writing();notes();papers();conversations();handoffs();};},[owners]);
  return <Context.Provider value={owners}>{children}</Context.Provider>;
}

export function useAgentSteeringOwners():AgentSteeringOwners|null{return useContext(Context);}
