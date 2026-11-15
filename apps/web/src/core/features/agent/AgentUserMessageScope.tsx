import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { agentDraftFromInput, decodeAgentUserInput, restoreAgentSourceDraft, type AgentInputDraft } from "@research-assistant/core/agent";
import type { WritingAgentMessage } from "./presentation/WritingAgentMessage";
import type { ReadMessageData } from "./AgentMessageData";
import { readMessageInspection } from "./messageInspectionGateway";

const DraftContext = createContext<{id: string; sourceText: string; draft: AgentInputDraft} | null>(null);

/** One frozen metadata read per message, shared by its body and copy action. */
export function useAgentUserMessageDraft(message: WritingAgentMessage, read?: ReadMessageData): AgentInputDraft {
  const shared = useContext(DraftContext);
  const provided = shared?.id === message.id && shared.sourceText === message.content ? shared.draft : undefined;
  const [loaded, setLoaded] = useState<{id: string; text: string; draft: AgentInputDraft} | null>(null);
  const hasRecord = message.rawData !== undefined;
  useEffect(() => {
    if (provided || message.role !== "user" || message.userInput !== undefined || message.streaming || !hasRecord) return;
    let active = true;
    const request = read ? () => read(message, {referencesOnly: true}) : message.conversationId && message.turnId
      ? () => readMessageInspection({conversationId: message.conversationId!, turnId: message.turnId!, itemId: message.id.split("/")[1]!, referencesOnly: true}) : undefined;
    if (!request) return;
    void Promise.resolve().then(request).then(record => {
      if (!record || typeof record !== "object" || !("userInput" in record)) return;
      const draft = agentDraftFromInput(message.content, decodeAgentUserInput(record.userInput, message.content));
      if (active) setLoaded({id: message.id, text: message.content, draft});
    }).catch(() => { /* Legacy or corrupt metadata stays literal text. */ });
    return () => { active = false; };
  }, [provided, message.id, message.content, message.role, message.streaming, message.userInput, message.conversationId, message.turnId, hasRecord, read]);
  if (provided) return provided;
  try {
    const draft = message.userInput !== undefined ? agentDraftFromInput(message.content, message.userInput)
      : loaded?.id === message.id && loaded.text === message.content ? loaded.draft : {text: message.content, bindings: []};
    return restoreAgentSourceDraft(draft);
  } catch { return {text: message.content, bindings: []}; }
}

export function AgentUserMessageScope({message, read, children}: {
  readonly message: WritingAgentMessage; readonly read: ReadMessageData | undefined; readonly children: ReactNode;
}): React.JSX.Element {
  const draft = useAgentUserMessageDraft(message, read);
  return <DraftContext.Provider value={message.role === "user" ? {id: message.id, sourceText: message.content, draft} : null}>{children}</DraftContext.Provider>;
}

export function useUserMessageCopyDraft(id: string | undefined): AgentInputDraft | undefined {
  const shared = useContext(DraftContext);
  return shared && shared.id === id ? shared.draft : undefined;
}
