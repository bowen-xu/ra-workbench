import { useEffect, useRef } from "react";

export interface AgentQueueAutoPrepareInput {
  readonly inputId: string;
  readonly text: string;
  readonly userInput?: readonly import("@research-assistant/core/agent").AgentUserInput[];
}

/** Automatically prepares the durable queue head whenever the host is idle.
 * The host still owns context/model validation and starts the model only after
 * its own preview gateway accepts the frozen submission. */
export function useAgentQueueAutoPrepare(
  running: boolean,
  queue: AgentQueueAutoPrepareInput | null,
  blocked: boolean,
  prepare: (input: AgentQueueAutoPrepareInput) => void | false,
): { readonly markQueued: () => void } {
  const requested = useRef(0);
  const preparedInputId = useRef<string | null>(null);
  const prepareRef = useRef(prepare);
  prepareRef.current = prepare;
  const markQueued = () => {
    requested.current += 1;
  };
  useEffect(() => {
    if (queue === null) {
      requested.current = 0;
      preparedInputId.current = null;
      return;
    }
    if (running || blocked || (requested.current === 0 && preparedInputId.current === queue.inputId)) return;
    requested.current = Math.max(0, requested.current - 1);
    preparedInputId.current = queue.inputId;
    // A reload effect can reserve the live owner after this render's blocked
    // snapshot. Don't latch an item that the host has not begun dispatching.
    if (prepareRef.current(queue) === false) preparedInputId.current = null;
  }, [blocked, queue, running]);
  useEffect(() => {
    if (!running && queue === null) requested.current = 0;
  }, [queue, running]);
  return { markQueued };
}
