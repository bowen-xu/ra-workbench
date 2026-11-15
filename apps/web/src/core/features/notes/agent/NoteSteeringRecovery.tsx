import {useCallback} from "react";
import type {AgentNoteScope} from "@research-assistant/core/agent";
import {agentNoteSteeringGateway} from "../../../services/agentGateway";
import {AgentSteeringRecovery,type AgentSteeringRecoveryProps} from "../../agent/AgentSteeringRecovery";
export function NoteSteeringRecovery(props:Omit<AgentSteeringRecoveryProps,"read"|"className">&{readonly scope:AgentNoteScope}):React.JSX.Element{
  const {conversationId,scope:{kind,resourceId}}=props.scope;
  const read=useCallback((turnId:string)=>agentNoteSteeringGateway.recover({conversationId,scope:{kind,resourceId},turnId}),[conversationId,kind,resourceId]);
  return <AgentSteeringRecovery {...props} read={read} className="note-agent-recovery"/>;
}
