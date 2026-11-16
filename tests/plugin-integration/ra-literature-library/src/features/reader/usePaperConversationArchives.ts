import type { Dispatch, MutableRefObject, SetStateAction } from "react";
import type { AgentConversationBinding, AgentConversationSummary } from "@research-assistant/core/agent";
import { agentArchiveGateway, agentMetadataGateway } from "../../../../../../apps/web/src/core/public/agent";
import { clearAgentSessionSelection } from "../../../../../../apps/web/src/core/public/agentConversations/agentSessionSelection";
import { useConversationArchives } from "../../../../../../apps/web/src/core/public/agentConversations/useConversationArchives";

interface Options {
  readonly binding: AgentConversationBinding;
  readonly conversationId: string | undefined;
  readonly archivedView: boolean;
  readonly selection: MutableRefObject<number>;
  readonly identity: MutableRefObject<{ paperId: string; conversationId: string | undefined }>;
  readonly setConversations: Dispatch<SetStateAction<readonly AgentConversationSummary[]>>;
  readonly selectConversation: (id: string | undefined) => void;
}
/** Paper selection is a view qualification, separate from the retained catalog operation. */
export function usePaperConversationArchives(options: Options) {
  const { binding, conversationId, archivedView, selection, identity, setConversations, selectConversation } = options;
  const operations = useConversationArchives(binding, { archives: agentArchiveGateway, metadata: agentMetadataGateway }, result => {
    setConversations(current => (current.some(entry => entry.id === result.id)
      ? current.map(entry => entry.id === result.id ? result : entry) : [result, ...current]).filter(entry => entry.archived === archivedView));
    if (identity.current.conversationId === result.id && result.archived !== archivedView) {
      clearAgentSessionSelection(binding, result.id); selectConversation(undefined);
    }
  }, undefined, () => {
    const revision = selection.current;
    return () => selection.current === revision;
  });
  return {
    pending: conversationId !== undefined && operations.pending.has(conversationId.toLowerCase()),
    change(conversation: AgentConversationSummary): Promise<void> {
      const revision = selection.current;
      return operations.change(conversation, () => selection.current === revision && identity.current.paperId === binding.resourceId
        && identity.current.conversationId === conversation.id);
    },
  };
}
