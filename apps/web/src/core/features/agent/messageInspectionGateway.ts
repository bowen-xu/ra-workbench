import { invoke } from "../../runtime/platformCore";
import { hasPlatformCapabilities } from "../../runtime/appRuntime";
import { decodeAgentEvent, type AgentEvent, type AgentUserInput } from "@research-assistant/core/agent";

export interface MessageInspectionRequest {
  readonly conversationId: string;
  readonly turnId: string;
  readonly itemId: string;
  readonly referencesOnly?: boolean;
}
export interface MessageInspectionData extends MessageInspectionRequest {
  readonly schemaVersion: 1;
  readonly events: readonly AgentEvent[];
  readonly frozenSubmission: unknown;
  readonly userInput?: readonly AgentUserInput[];
}
export type ReadMessageInspection = (request: MessageInspectionRequest) => Promise<MessageInspectionData>;
export const readMessageInspection: ReadMessageInspection = async request => {
  if (!hasPlatformCapabilities()) throw new Error("原始数据需要桌面应用或模拟网关");
  const value = await invoke<MessageInspectionData>("agent_read_message_data", { request });
  if (value.schemaVersion !== 1 || value.conversationId !== request.conversationId || value.turnId !== request.turnId
    || value.itemId !== request.itemId || !Array.isArray(value.events)) throw new Error("原始数据身份不匹配");
  const events = value.events.map(decodeAgentEvent);
  if (events.some(event => event.conversationId !== request.conversationId || event.payload.turnId !== request.turnId
    || !("itemId" in event.payload) || event.payload.itemId !== request.itemId)) throw new Error("原始数据身份不匹配");
  return { ...value, events };
};
