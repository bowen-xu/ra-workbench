import { createContext, useContext, type ComponentType, type ReactNode } from "react";
import type { AgentConversationSummary } from "@research-assistant/core/agent";
import type { AgentCanvasMessage } from "../AgentCanvasMessage";
import type { AgentContextDirectory } from "./contextDirectoryGateway";

/** The Core conversation owns selection and history identity. An application
 * supplies the domain execution view without transferring its storage owner. */
export interface AgentDomainSessionProps {
  readonly messageRequest?: AgentCanvasMessage | null;
  readonly id: string;
  readonly directory: AgentContextDirectory;
  readonly title: string;
  readonly archived: boolean;
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly headerControls: ReactNode;
  readonly onRenamed: (row: AgentConversationSummary) => void;
}

interface ConversationAdapters {
  readonly WritingSession?: ComponentType<AgentDomainSessionProps>;
}

const Context = createContext<ConversationAdapters>({});

export function AgentConversationAdaptersProvider({ WritingSession, children }: ConversationAdapters & {readonly children: ReactNode}): React.JSX.Element {
  return <Context value={WritingSession ? {WritingSession} : {}}>{children}</Context>;
}

export function useWritingConversationSession(): ComponentType<AgentDomainSessionProps> | undefined {
  return useContext(Context).WritingSession;
}
