import type {MutableRefObject} from "react";
import { agentTurnGateway } from "../../../../../../apps/web/src/core/public/agent";
import {aiAssistanceErrorMessage} from "@research-assistant/workbench-sdk/ai-assistance";
import {notify} from "@research-assistant/workbench-sdk/feedback";

export async function interruptPaperTurn(active: {readonly conversationId: string; readonly turnId: string} | undefined,
  selection: MutableRefObject<number>, setError: (value: string | undefined) => void): Promise<void> {
  if (!active) return;
  const revision = selection.current;
  try {await agentTurnGateway.interrupt(active); notify.info("已请求中断当前回答");}
  catch (reason: unknown) {
    if (selection.current !== revision) return;
    const message = aiAssistanceErrorMessage(reason); setError(message); notify.error(message);
  }
}
