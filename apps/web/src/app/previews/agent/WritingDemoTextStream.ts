export interface WritingDemoTextPacing {
  readonly charactersPerChunk: number;
  readonly intervalMs: number;
}
export const DEFAULT_DEMO_TEXT_PACING: WritingDemoTextPacing = { charactersPerChunk: 3, intervalMs: 300 };

/** Mock provider output cursor. Live pacing changes never rewind emitted text. */
export class WritingDemoTextStream {
  private offset = 0;
  private replayPrefix = false;
  private readonly characters: readonly string[];
  constructor(text: string, private readonly pacing: () => WritingDemoTextPacing = () => DEFAULT_DEMO_TEXT_PACING) {
    this.characters = Array.from(text);
  }
  get remainingSteps(): number {
    return Math.ceil((this.characters.length - this.offset) / this.pacing().charactersPerChunk);
  }
  get intervalMs(): number { return this.pacing().intervalMs; }
  get hasOutput(): boolean { return this.offset > 0; }
  /** Mock recovery immediately restores the received prefix; pacing applies
   * only to new text, rather than simulating a second slow replay. */
  restart(): void { this.replayPrefix = true; }
  next(): string {
    const end = Math.min(this.characters.length, this.offset + this.pacing().charactersPerChunk);
    const text = this.characters.slice(this.replayPrefix ? 0 : this.offset, end).join("");
    this.replayPrefix = false;
    this.offset = end;
    return text;
  }
}
