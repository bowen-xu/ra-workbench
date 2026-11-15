export type SpeechInputState = "idle" | "starting" | "listening" | "stopping";

export interface SpeechRecognitionAlternativeLike {
  readonly transcript: string;
}

export interface SpeechRecognitionResultLike {
  readonly length: number;
  readonly isFinal: boolean;
  readonly [index: number]: SpeechRecognitionAlternativeLike;
}

export interface SpeechRecognitionResultListLike {
  readonly length: number;
  readonly [index: number]: SpeechRecognitionResultLike;
}

export interface SpeechRecognitionResultEventLike {
  readonly resultIndex: number;
  readonly results: SpeechRecognitionResultListLike;
}

export interface SpeechRecognitionErrorEventLike {
  readonly error: string;
  readonly message?: string;
}

export interface SpeechRecognitionLike {
  continuous: boolean;
  interimResults: boolean;
  lang: string;
  maxAlternatives: number;
  onend: (() => void) | null;
  onerror: ((event: SpeechRecognitionErrorEventLike) => void) | null;
  onresult: ((event: SpeechRecognitionResultEventLike) => void) | null;
  onstart: (() => void) | null;
  abort(): void;
  start(): void;
  stop(): void;
}

export type SpeechRecognitionConstructor = new () => SpeechRecognitionLike;

export interface SpeechInputFailure {
  readonly level: "error" | "warning";
  readonly message: string;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

export function getSpeechRecognitionConstructor(value: unknown): SpeechRecognitionConstructor | undefined {
  if (!isRecord(value)) return undefined;
  const constructor = typeof value.webkitSpeechRecognition === "function"
    ? value.webkitSpeechRecognition
    : value.SpeechRecognition;
  if (typeof constructor !== "function") return undefined;
  return constructor as SpeechRecognitionConstructor;
}

export function speechRecognitionTranscript(event: SpeechRecognitionResultEventLike): string {
  let transcript = "";
  for (let index = 0; index < event.results.length; index += 1) {
    const part = event.results[index]?.[0]?.transcript.trim();
    if (part === undefined || part === "") continue;
    transcript = `${transcript}${needsSeparator(transcript, part) ? " " : ""}${part}`;
  }
  return transcript;
}

function needsSeparator(left: string, right: string): boolean {
  if (left === "" || right === "" || /\s$/u.test(left) || /^\s/u.test(right)) return false;
  if (/^[,.;:!?，。；：！？、)\]}]/u.test(right)) return false;
  const leftCharacter = left.at(-1) ?? "";
  const rightCharacter = right.at(0) ?? "";
  const cjk = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/u;
  return !(cjk.test(leftCharacter) && cjk.test(rightCharacter));
}

export function mergeSpeechTranscript(current: string, previousSpeech: string, nextSpeech: string): string {
  const previousIndex = previousSpeech === "" ? -1 : current.lastIndexOf(previousSpeech);
  if (previousIndex >= 0) {
    return `${current.slice(0, previousIndex)}${nextSpeech}${current.slice(previousIndex + previousSpeech.length)}`;
  }
  if (nextSpeech === "") return current;
  return `${current}${needsSeparator(current, nextSpeech) ? " " : ""}${nextSpeech}`;
}

export function speechInputFailure(error: string): SpeechInputFailure | undefined {
  switch (error) {
    case "aborted":
      return undefined;
    case "no-speech":
      return { level: "warning", message: "没有检测到语音，请靠近麦克风后重试。" };
    case "audio-capture":
      return { level: "error", message: "未检测到可用的麦克风，请检查系统音频输入设备。" };
    case "not-allowed":
    case "service-not-allowed":
      return {
        level: "error",
        message: "无法使用语音输入，请在系统设置中允许 Research Assistant 访问麦克风和语音识别。",
      };
    case "network":
      return { level: "error", message: "语音识别服务连接失败，请检查网络后重试。" };
    case "language-not-supported":
    case "language-unavailable":
      return { level: "error", message: "系统语音识别不支持当前语言，请调整系统听写语言后重试。" };
    default:
      return { level: "error", message: "语音识别失败，请稍后重试。" };
  }
}
