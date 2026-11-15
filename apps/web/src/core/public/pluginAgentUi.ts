/** Finite Agent UI surface used by existing literature consumers. Every value
 * retains its original Core component, context and resource owner. */
export {SelectionAskAiButton, SelectionAskAiEditor} from "./agentContext/SelectionAskAi";
export {submitEvidenceAnnotation} from "./agentContext/submitEvidenceAnnotation";
export {openAgentConversation} from "./agentConversations/openAgentConversation";
export {useAgentCanvasMessage} from "./agentPresentation/AgentCanvasShell";
export {captureEvidenceImage} from "./agentPresentation/evidenceImageGateway";
export type {AgentCanvasMessage, AgentCanvasMessageOptions} from "./agentPresentation/AgentCanvasMessage";
