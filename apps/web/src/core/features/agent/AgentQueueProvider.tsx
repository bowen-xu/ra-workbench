import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { AgentInputQueueRegistry } from "@research-assistant/data/agent";
import { agentInputQueueGateway, agentNoteQueueGateway, agentPaperQueueGateway } from "../../services/agentGateway";
import {agentContextGateway} from "../../services/agentContextGateway";
import type {AgentQueueState, AgentQueueCommand} from "@research-assistant/core/agent";
import {captureQueueOwnerIntent} from "./captureQueueOwnerIntent";

function createOwners() {
  return {
    conversations: new AgentInputQueueRegistry<{conversationId:string}, AgentQueueState>({
      read: scope => agentContextGateway.queue!.read(scope),
      edit: (request: {conversationId:string} & AgentQueueCommand) => agentContextGateway.queue!.edit(request),
      inspectEdit: request => {
        const inspect = agentContextGateway.queue?.inspectEdit;
        if (!inspect) return Promise.reject(new Error("CONTEXT_QUEUE_EDIT_STATUS_UNAVAILABLE"));
        return inspect(request);
      },
    }, scope => scope.conversationId, () => crypto.randomUUID(), captureQueueOwnerIntent),
    writing: new AgentInputQueueRegistry(agentInputQueueGateway,
      scope => JSON.stringify([scope.manuscriptId, scope.conversationId]), () => crypto.randomUUID(), captureQueueOwnerIntent),
    notes: new AgentInputQueueRegistry(agentNoteQueueGateway,
      scope => JSON.stringify([scope.scope.kind, scope.scope.resourceId, scope.conversationId]), () => crypto.randomUUID(), captureQueueOwnerIntent),
    papers: new AgentInputQueueRegistry(agentPaperQueueGateway,
      scope => JSON.stringify([scope.paperId, scope.conversationId]), () => crypto.randomUUID(), captureQueueOwnerIntent),
  };
}
const Context = createContext<ReturnType<typeof createOwners> | null>(null);
export function AgentQueueProvider({ children }: { readonly children: ReactNode }): React.JSX.Element {
  const [owners] = useState(createOwners);
  useEffect(() => {
    const writing = owners.writing.retain(), notes = owners.notes.retain(), papers = owners.papers.retain(), conversations = owners.conversations.retain();
    return () => { writing(); notes(); papers(); conversations(); };
  }, [owners]);
  return <Context.Provider value={owners}>{children}</Context.Provider>;
}
export function useAgentQueueOwners() { return useContext(Context); }
