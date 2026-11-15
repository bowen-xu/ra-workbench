import { getAppStorage } from "../../runtime/appStorage";
import type { AgentConversationBinding } from "@research-assistant/core/agent";

const PREFIX = "research-assistant.agent-session.";

function key(binding: AgentConversationBinding): string {
  return `${PREFIX}${binding.kind}.${binding.resourceId}`;
}

export function readAgentSessionSelection(binding: AgentConversationBinding): string | null {
  try {
    const value = getAppStorage().getItem(key(binding));
    return value && value.trim() ? value : null;
  } catch {
    return null;
  }
}

export function writeAgentSessionSelection(binding: AgentConversationBinding, conversationId: string): void {
  try {
    getAppStorage().setItem(key(binding), conversationId);
  } catch {
    // Local storage is an optional convenience; session selection remains usable.
  }
}

export function clearAgentSessionSelection(binding: AgentConversationBinding, conversationId?: string): void {
  try {
    const storageKey = key(binding);
    if (!conversationId || getAppStorage().getItem(storageKey) === conversationId) {
      getAppStorage().removeItem(storageKey);
    }
  } catch {
    // Local storage is an optional convenience; session selection remains usable.
  }
}
