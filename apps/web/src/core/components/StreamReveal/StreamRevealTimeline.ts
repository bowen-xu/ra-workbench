export const STREAM_REVEAL_DURATION_MS = 300;

/** A source-text frontier: extending a stream preserves its fractional progress. */
export class StreamRevealTimeline {
  private from: number;
  private target: number;
  private startedAt: number;
  constructor(private value: string, private streaming: boolean, now: number,
    private durationMs = STREAM_REVEAL_DURATION_MS) {
    this.durationMs = this.validDuration(durationMs);
    this.from = streaming ? 0 : value.length;
    this.target = value.length;
    this.startedAt = now;
  }
  position(now: number): number {
    const fraction = this.durationMs === 0 ? 1 : Math.min(1, Math.max(0, (now - this.startedAt) / this.durationMs));
    return this.from + (this.target - this.from) * fraction;
  }
  update(value: string, streaming: boolean, now: number, durationMs = this.durationMs): void {
    const duration = this.validDuration(durationMs);
    if (value !== this.value || duration !== this.durationMs) {
      const append = value.startsWith(this.value) && (this.streaming || streaming);
      this.from = append || value === this.value ? this.position(now) : value.length;
      this.target = value.length;
      this.startedAt = now;
      this.value = value;
      this.durationMs = duration;
    }
    this.streaming = streaming;
  }
  settle(): void { this.from = this.target; }
  get end(): number { return this.target; }
  private validDuration(value: number): number {
    return Number.isFinite(value) && value >= 0 ? value : STREAM_REVEAL_DURATION_MS;
  }
}
