import {useAgentSteeringOwners} from "../../agent/AgentSteeringProvider";
import type {AgentConversationProjection,AgentNoteSteeringScope,AgentNoteSteerRequest} from "@research-assistant/core/agent";
import {agentNoteSteeringGateway} from "../../../services/agentGateway";
import {useAgentSteering} from "../../agent/useAgentSteering";
const send=(request:AgentNoteSteerRequest)=>agentNoteSteeringGateway.steer(request);
export function useNoteSteering(scope:AgentNoteSteeringScope|null,accepted:(text:string)=>void,projection:AgentConversationProjection|null=null,host?:{readonly key:string;readonly conversationId:string;readonly queueRejected?:import("../../agent/useAgentSteering").AgentSteeringSubmitCallbacks["queueRejected"];readonly captureAcknowledgment:(text:string)=>()=>void},historyGate?:{readonly isReady:()=>boolean;readonly state:unknown}){
  const owners=useAgentSteeringOwners();
  return useAgentSteering(scope,scope?`${scope.conversationId}/${scope.scope.kind}/${scope.scope.resourceId}/${scope.turnId}`:"",send,accepted,projection,owners&&host?{registry:owners.notes,hostKey:host.key,conversationId:host.conversationId,captureAcknowledgment:host.captureAcknowledgment,...(host.queueRejected?{queueRejected:host.queueRejected}:{})}:undefined,historyGate);
}
