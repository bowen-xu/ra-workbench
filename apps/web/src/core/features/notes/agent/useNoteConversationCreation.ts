import { useRef } from "react";
import type { AgentConversationBinding, AgentConversationSummary } from "@research-assistant/core/agent";
import type {ConversationDraftStore} from "@research-assistant/workbench-sdk/agent-drafts";
import { useBoundConversationCreation } from "../../agent/useBoundConversationCreation";
import { useAgentHostLifecycle, useAgentMountedGuard } from "../../agent/useAgentMountedGuard";
import { agentCreationGateway, agentMetadataGateway } from "../../../services/agentGateway";

export function useNoteConversationCreation(options: { readonly binding: AgentConversationBinding; readonly conversationId: string;
  readonly pending: boolean; readonly saved: boolean; readonly store: ConversationDraftStore;
  readonly onCreated?: (conversation: AgentConversationSummary) => void }) {
  const { binding, conversationId, pending, store } = options;
  const lifecycle = useAgentHostLifecycle([binding.kind, binding.resourceId, conversationId]);
  const mounted = useAgentMountedGuard();
  const current = useRef(options); current.current = options;
  const creation = useBoundConversationCreation(binding, { creation: agentCreationGateway, metadata: agentMetadataGateway });
  function adopt(conversation: AgentConversationSummary | null, epoch = lifecycle.current): boolean {
    const scope = current.current;
    if (!mounted.current || !conversation || conversation.archived || !scope.pending || !scope.saved || scope.conversationId !== conversation.id
      || lifecycle.current !== epoch) return false;
    return creation.consumeAdoption(() => {
      store.adoptPending({ hostKind: binding.kind, hostId: binding.resourceId }, conversation.id);
      scope.onCreated?.(conversation); return true;
    });
  }
  async function ensureConversation(): Promise<boolean> {
    if (!pending) return true;
    const epoch = lifecycle.current; return adopt(await creation.submit(conversationId), epoch);
  }
  const captureAdoption = () => { const epoch = lifecycle.current; return (conversation: AgentConversationSummary | null) => { adopt(conversation, epoch); }; };
  return { creation, ensureConversation, captureAdoption };
}
