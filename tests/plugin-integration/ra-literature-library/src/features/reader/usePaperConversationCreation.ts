import { useRef, type MutableRefObject } from "react";
import type { AgentConversationSummary } from "@research-assistant/core/agent";
import { agentCreationGateway, agentMetadataGateway } from "../../../../../../apps/web/src/core/public/agent";
import { useBoundConversationCreation } from "../../../../../../apps/web/src/core/public/agentConversations/useBoundConversationCreation";
import { useAgentMountedGuard } from "../../../../../../apps/web/src/core/public/agentPresentation/useAgentMountedGuard";

export function usePaperConversationCreation(options: { readonly paperId: string; readonly conversationId: string | undefined;
  readonly epoch: MutableRefObject<number>; readonly selectionRevision: MutableRefObject<number>;
  readonly onCreated: (conversation: AgentConversationSummary) => void }) {
  const creation = useBoundConversationCreation({ kind: "paper", resourceId: options.paperId }, { creation: agentCreationGateway, metadata: agentMetadataGateway });
  const mounted = useAgentMountedGuard();
  const scope = useRef(options); scope.current = options;
  function adopt(conversation: AgentConversationSummary | null, epoch = options.epoch.current, revision = options.selectionRevision.current): boolean {
    const current = scope.current;
    if (!mounted.current || !conversation || conversation.archived || current.conversationId !== undefined || current.paperId !== options.paperId
      || current.epoch.current !== epoch || current.selectionRevision.current !== revision) return false;
    return creation.consumeAdoption(() => { current.onCreated(conversation); return true; });
  }
  async function createConversation(epoch: number, id?: string): Promise<string | undefined> {
    const revision = options.selectionRevision.current, current = await creation.submit(id);
    return adopt(current, epoch, revision) ? current!.id : undefined;
  }
  const captureAdoption = () => {
    const epoch = options.epoch.current, revision = options.selectionRevision.current;
    return (conversation: AgentConversationSummary | null) => { adopt(conversation, epoch, revision); };
  };
  return { creation, createConversation, captureAdoption };
}
