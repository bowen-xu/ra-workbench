import type { WritingDiffDemoHistory } from "./WritingDiffDemoHistory";
import type { WritingSubmissionAdmission } from "../../../core/features/agent/writingSubmissionController";
import { WritingDemoTextStream, type WritingDemoTextPacing } from "./WritingDemoTextStream";

/** One final-answer item; only gateway events are mocked, never UI markup. */
export class WritingDemoSingleText {
  private started = false;
  private itemId = crypto.randomUUID();
  private readonly textStream: WritingDemoTextStream;
  constructor(text: string, pacing?: () => WritingDemoTextPacing) { this.textStream = new WritingDemoTextStream(text, pacing); }
  restartIncompleteResponse(): void {
    if (!this.started || !this.remainingSteps) return;
    this.started = false;
    this.itemId = crypto.randomUUID();
    this.textStream.restart();
  }
  get remainingSteps(): number { return this.textStream.remainingSteps; }
  get isStreaming(): boolean { return this.started && this.remainingSteps > 0; }
  nextDelayMs(waitMs: number): number { return this.started || this.textStream.hasOutput ? this.textStream.intervalMs : waitMs; }
  next(history: WritingDiffDemoHistory, admission: WritingSubmissionAdmission): boolean {
    const { turnId } = admission, itemId = this.itemId;
    const text = this.textStream.next();
    history.feedbackEvents(admission, [
      ...(!this.started ? [
        { type: "itemStarted" as const, turnId, itemId, kind: { type: "assistantMessage" as const } },
        { type: "itemPhaseChanged" as const, turnId, itemId, phase: "finalAnswer" as const },
      ] : []),
      { type: "textDelta", turnId, itemId, text },
      ...(!this.remainingSteps ? [{ type: "itemCompleted" as const, turnId, itemId }] : []),
    ]);
    this.started = true;
    return text.length > 0;
  }
}
