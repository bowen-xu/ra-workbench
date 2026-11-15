import { decodeAgentContextState, EMPTY_AGENT_CONTEXT, type AgentContextState } from "@research-assistant/core/agent";
import { getAppStorage } from "../../../runtime/appStorage";
const CHANGED = "research-assistant:conversation-context-changed";
export function contextStorageKey(id: string): string { return `research-assistant:agent-context:v1:${id}`; }
export function hasConversationContext(id:string):boolean {return getAppStorage().getItem(contextStorageKey(id)) !== null;}
export function readConversationContext(id: string): AgentContextState {
    const raw = getAppStorage().getItem(contextStorageKey(id));
    return raw === null ? EMPTY_AGENT_CONTEXT : decodeAgentContextState(JSON.parse(raw));
}
export function writeConversationContext(id: string, state: AgentContextState): void {
    getAppStorage().setItem(contextStorageKey(id), JSON.stringify(decodeAgentContextState(state)));
    window.dispatchEvent(new CustomEvent(CHANGED, {detail: {id}}));
}
/** Independent and writing hosts can remain mounted for the same conversation. */
export function subscribeConversationContext(id: string, changed: () => void): () => void {
    const listener = (event: Event): void => {if ((event as CustomEvent<{id:string}>).detail.id === id) changed();};
    window.addEventListener(CHANGED, listener);
    return () => window.removeEventListener(CHANGED, listener);
}
