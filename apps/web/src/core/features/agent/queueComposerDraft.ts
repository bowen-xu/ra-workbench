import { decodeAgentQueueState, type AgentInputDraft, type AgentQueueEdit, type AgentQueueState, type AgentUserInput } from "@research-assistant/core/agent";

export interface QueueDraftPlacement {
  readonly input: AgentQueueState["inputs"][number];
  readonly index: number;
  readonly preceding: readonly string[];
  readonly following: readonly string[];
}
export interface ConversationInputDraft extends AgentInputDraft { readonly queuePlacement?: QueueDraftPlacement }

/** Persistence validates the original complete payload as strictly as the IPC queue. */
export function decodeQueueDraftPlacement(value: unknown): QueueDraftPlacement {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("无效的队列草稿");
  const row = value as Record<string, unknown>;
  if (Object.keys(row).sort().join() !== "following,index,input,preceding" || typeof row.index !== "number"
    || !Number.isSafeInteger(row.index) || row.index < 0 || row.index >= 64) throw new Error("无效的队列草稿位置");
  const queue = decodeAgentQueueState({schemaVersion: 1, conversationId: "00000000-0000-0000-0000-000000000001",
    revision: 0, paused: false, preparingInputId: null, inputs: [row.input]});
  const input = queue.inputs[0];
  if (!input) throw new Error("无效的队列草稿");
  function ids(value: unknown): readonly string[] {
    if (!Array.isArray(value) || value.length > 64 || value.some(id => typeof id !== "string" || !/^[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i.test(id) || /^0{8}(-0{4}){3}-0{12}$/.test(id))) throw new Error("无效的队列草稿邻居");
    return value as string[];
  }
  return {input, index: row.index, preceding: ids(row.preceding), following: ids(row.following)};
}

/** Anchor to remaining neighbours when earlier inputs have already been dispatched. */
export function queueDraftEdit(draft: ConversationInputDraft, queue: AgentQueueState, text: string,
  userInput: readonly AgentUserInput[], newId: () => string = () => crypto.randomUUID()): AgentQueueEdit {
  const placement = draft.queuePlacement;
  if (!placement) return {type: "add", inputId: newId(), text, userInput};
  // A proven removal rejection leaves the original intact; updating it is atomic.
  if (queue.inputs.some(input => input.inputId === placement.input.inputId)) return {type: "update", inputId: placement.input.inputId, text, userInput};
  const following = placement.following.map(id => queue.inputs.findIndex(input => input.inputId === id)).find(index => index >= 0);
  const preceding = [...placement.preceding].reverse().map(id => queue.inputs.findIndex(input => input.inputId === id)).find(index => index >= 0);
  const index = following ?? (preceding === undefined ? Math.min(placement.index, queue.inputs.length) : preceding + 1);
  return {type: "restore", inputId: newId(), sourceInputId: placement.input.inputId, index, text, userInput};
}
