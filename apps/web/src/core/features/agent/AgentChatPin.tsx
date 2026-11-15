import {createContext, useContext, useMemo, useState, type Dispatch, type ReactNode, type SetStateAction} from "react";
import {useWorkspaceState,workspaceBoolean} from "../../runtime/workspaceState";

type ChatPinState = readonly [boolean, Dispatch<SetStateAction<boolean>>];
const ChatPinContext = createContext<ChatPinState | null>(null);

/** The pin belongs to the chat workspace, across conversation and adapter changes. */
export function AgentChatPinProvider({children}: {readonly children: ReactNode}): React.JSX.Element {
  const [pinned, setPinned] = useWorkspaceState("chat-pinned",false,workspaceBoolean);
  const state = useMemo<ChatPinState>(() => [pinned, setPinned], [pinned]);
  return <ChatPinContext value={state}>{children}</ChatPinContext>;
}

/** Standalone panels retain their own pin when there is no workspace host. */
export function useAgentChatPin(): ChatPinState {
  const shared = useContext(ChatPinContext);
  const local = useState(false);
  return shared ?? local;
}
