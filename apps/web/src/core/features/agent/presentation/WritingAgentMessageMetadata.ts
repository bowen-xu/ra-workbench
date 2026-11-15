import type { WritingAgentMessage } from "./WritingAgentMessage";

/** Each durable turn has one response footer, after its last assistant text.
 * Legacy messages without turn identity use user-message boundaries. */
export function assistantMetadataIds(messages: readonly WritingAgentMessage[], isRunning: boolean): ReadonlySet<string> {
  const responses = new Map<string, { message: WritingAgentMessage; finished?: boolean }>();
  let legacyTurn = 0;
  let latestTurn: string | undefined;
  for (const message of messages) {
    if (!message.turnId && message.role === "user") legacyTurn += 1;
    const turn = message.turnId ? `turn:${message.turnId}` : `legacy:${legacyTurn}`;
    latestTurn = turn;
    // Independent summaries and tools belong to the process, never the reply footer.
    if (message.role === "assistant" && !message.failure && message.kind === undefined) {
      const previous = responses.get(turn)?.message;
      if (message.phase === "finalAnswer" || previous?.phase !== "finalAnswer") {
        responses.set(turn, { message, ...(message.turnFinished === undefined ? {} : { finished: message.turnFinished }) });
      }
    }
  }
  return new Set([...responses].flatMap(([turn, { message, finished }]) =>
    !message.streaming && (finished ?? (!isRunning || turn !== latestTurn)) ? [message.id] : []));
}
