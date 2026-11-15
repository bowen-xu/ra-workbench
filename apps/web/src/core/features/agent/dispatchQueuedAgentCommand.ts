import { readComposerCommand } from "./agentComposerCommands";
import type { AgentCommandExecution } from "./useAgentCommandExecution";
import { notify } from "../../services/notifications";
import type { AgentQueueState } from "@research-assistant/core/agent";
import type { AgentInputQueueState } from "@research-assistant/data/agent";

export const compactRunningMessage = "当前任务运行中，不能引导 /compact；请等待当前任务结束后执行。";
type CommandTransfer = { apply: () => Promise<boolean>; release: () => void };

/** Check the live queue, not a stale row/event captured before an edit. */
export function queuedCommandTransfer(controller: {readonly current: {getSnapshot: () => AgentInputQueueState<AgentQueueState>} | null},
  inputId: string, text: string, retain: () => CommandTransfer | null): CommandTransfer | null {
  const state = controller.current?.getSnapshot();
  if (!state || state.loading || state.editing || state.uncertain || state.queue?.preparingInputId
    || !state.queue?.inputs.some(input => input.inputId === inputId && input.text === text)) return null;
  return retain();
}

/** null means ordinary input. Commands never enter message preparation/steering.
 * Keep the durable item until completion; an uncertain removal uses the queue's
 * existing receipt recovery rather than automatically executing it again. */
export async function dispatchQueuedAgentCommand({ inputId, text, running, execution, transfer, executeQueued }: {
  readonly inputId: string;
  readonly text: string;
  readonly running: boolean;
  readonly execution?: Pick<AgentCommandExecution, "compact">;
  readonly transfer?: () => { apply: () => Promise<boolean>; release: () => void } | null;
  /** Native queued compaction owns its claim/settle, separate from generic removal. */
  readonly executeQueued?: () => Promise<boolean>;
}): Promise<boolean | null> {
  const command = readComposerCommand(text);
  if (!command) return null;
  if (running) { notify.error(compactRunningMessage); return false; }
  if (command.name !== "compact" || command.arguments) {
    notify.error(command.name === "compact" ? "/compact 不接受参数，请只输入 /compact。" : `未知命令：/${command.name}。`);
    return false;
  }
  if (executeQueued) return executeQueued();
  if (!execution || !transfer) return false;
  const owner = transfer();
  if (!owner) return false;
  try {
    if (!(await execution.compact(text, {preserveDraft: true, requestId: inputId}))) return false;
    if (!(await owner.apply())) {
      notify.error("上下文已压缩，但队列项移除未确认，请先核对队列，勿重复执行。");
      return false;
    }
    return true;
  } finally { owner.release(); }
}
