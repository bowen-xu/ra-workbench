import type { WritingDiffDemoHistory } from "./WritingDiffDemoHistory";
import type { WritingSubmissionAdmission } from "../../../core/features/agent/writingSubmissionController";
import { WritingDemoTextStream, type WritingDemoTextPacing } from "./WritingDemoTextStream";

/** Timed provider/tool events, consumed by the production history projection. */
export class WritingDemoFeedback {
  static readonly waitSteps = 3;
  private stage = 0;
  private readonly textStream: WritingDemoTextStream;
  private readonly callId = crypto.randomUUID();
  private readonly itemId = crypto.randomUUID();
  private messageId = crypto.randomUUID();
  constructor(private readonly step: {
    readonly name: string; readonly arguments: unknown; readonly output: unknown; readonly text: string;
    readonly phase?: "commentary" | "finalAnswer";
    readonly isError?: boolean;
  }, pacing?: () => WritingDemoTextPacing) { this.textStream = new WritingDemoTextStream(step.text, pacing); }
  restartIncompleteResponse(): void {
    if (!this.isStreaming) return;
    this.stage = 2; // Tool call/result already completed; regenerate only text.
    this.messageId = crypto.randomUUID();
    this.textStream.restart();
  }
  get remainingSteps(): number {
    return Math.max(0, 2 - this.stage) + (this.stage <= 2 ? Math.max(1, this.textStream.remainingSteps) : this.textStream.remainingSteps);
  }
  get remainingWaitSteps(): number { return this.textStream.hasOutput ? 0 : Math.max(0, WritingDemoFeedback.waitSteps - this.stage); }
  get isStreaming(): boolean { return this.stage >= WritingDemoFeedback.waitSteps && this.remainingSteps > 0; }
  get atModelBoundary(): boolean { return this.stage === 2 || (this.stage > 2 && this.remainingSteps === 0); }
  // Model thinking before the call, tool execution, then model thinking before text.
  nextDelayMs(waitMs: number): number { return this.remainingWaitSteps ? waitMs : this.textStream.intervalMs; }
  next(history: WritingDiffDemoHistory, admission: WritingSubmissionAdmission): boolean {
    if (!this.remainingSteps) return false;
    const { turnId } = admission;

    if (this.stage === 0) history.feedbackEvents(admission, [
      { type: "phaseChanged", turnId, phase: "runningTools" },
      { type: "itemStarted", turnId, itemId: this.itemId, kind: { type: "toolCall", callId: this.callId, name: this.step.name } },
      { type: "textDelta", turnId, itemId: this.itemId, text: JSON.stringify(this.step.arguments) },
    ]);
    else if (this.stage === 1) {
      const resultId = crypto.randomUUID();
      history.feedbackEvents(admission, [
        { type: "itemCompleted", turnId, itemId: this.itemId },
        { type: "itemStarted", turnId, itemId: resultId, kind: { type: "toolResult", callId: this.callId, ...(this.step.isError ? { isError: true } : {}) } },
        { type: "textDelta", turnId, itemId: resultId, text: JSON.stringify(this.step.output) },
        { type: "itemCompleted", turnId, itemId: resultId },
        { type: "phaseChanged", turnId, phase: "runningModel" },
      ]);
    } else {
      const text = this.textStream.next();
      history.feedbackEvents(admission, [
        ...(this.stage === 2 ? [{ type: "itemStarted" as const, turnId, itemId: this.messageId, kind: { type: "assistantMessage" as const } }] : []),
        ...(this.stage === 2 && this.step.phase ? [{ type: "itemPhaseChanged" as const, turnId, itemId: this.messageId, phase: this.step.phase }] : []),
        { type: "textDelta", turnId, itemId: this.messageId, text },
        ...(!this.textStream.remainingSteps ? [{ type: "itemCompleted" as const, turnId, itemId: this.messageId }] : []),
      ]);
    }
    const output = this.stage >= 2;
    this.stage++;
    return output;
  }
}
