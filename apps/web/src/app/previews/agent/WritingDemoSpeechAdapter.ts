import type { SpeechInputAdapter } from "../../../core/features/speech/SpeechInputAdapter";
import type { SpeechRecognitionLike, SpeechRecognitionResultEventLike, SpeechRecognitionErrorEventLike } from "../../../core/features/speech/speechRecognition";

/** Synthetic interim/final events; no microphone, permissions or network. */
class WritingDemoRecognition implements SpeechRecognitionLike {
  continuous = false;
  interimResults = false;
  lang = "";
  maxAlternatives = 1;
  onend: (() => void) | null = null;
  onerror: ((event: SpeechRecognitionErrorEventLike) => void) | null = null;
  onresult: ((event: SpeechRecognitionResultEventLike) => void) | null = null;
  onstart: (() => void) | null = null;
  private timers: ReturnType<typeof setTimeout>[] = [];
  start(): void {
    this.clear();
    this.onstart?.();
    this.timers.push(setTimeout(() => this.result("模拟语音输入：请帮我", false), 400));
    this.timers.push(setTimeout(() => this.result("模拟语音输入：请帮我完善摘要部分。", true), 900));
  }
  stop(): void { this.clear(); this.onend?.(); }
  abort(): void { this.clear(); }
  private clear(): void { for (const timer of this.timers) clearTimeout(timer); this.timers = []; }
  private result(transcript: string, isFinal: boolean): void {
    this.onresult?.({ resultIndex: 0, results: { length: 1, 0: { length: 1, isFinal, 0: { transcript } } } });
  }
}
export const writingDemoSpeechAdapter: SpeechInputAdapter = { recognition: WritingDemoRecognition };
