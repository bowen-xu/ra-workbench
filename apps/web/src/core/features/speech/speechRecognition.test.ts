import { describe, expect, it } from "vitest";

import {
  getSpeechRecognitionConstructor,
  mergeSpeechTranscript,
  speechInputFailure,
  speechRecognitionTranscript,
  type SpeechRecognitionResultEventLike,
} from "./speechRecognition";

class StandardRecognition {}
class WebkitRecognition {}

describe("paper reading speech recognition", () => {
  it("prefers the WebKit constructor and rejects invalid runtime values", () => {
    expect(getSpeechRecognitionConstructor({
      SpeechRecognition: StandardRecognition,
      webkitSpeechRecognition: WebkitRecognition,
    })).toBe(WebkitRecognition);
    expect(getSpeechRecognitionConstructor({ SpeechRecognition: StandardRecognition })).toBe(StandardRecognition);
    expect(getSpeechRecognitionConstructor({ SpeechRecognition: "invalid" })).toBeUndefined();
    expect(getSpeechRecognitionConstructor(undefined)).toBeUndefined();
  });

  it("reads the current recognition transcript and replaces interim text in place", () => {
    const event = {
      resultIndex: 0,
      results: {
        0: { 0: { transcript: "解释" }, isFinal: true, length: 1 },
        1: { 0: { transcript: "这篇论文" }, isFinal: false, length: 1 },
        length: 2,
      },
    } satisfies SpeechRecognitionResultEventLike;

    expect(speechRecognitionTranscript(event)).toBe("解释这篇论文");
    expect(mergeSpeechTranscript("已有内容 解释", "解释", "解释这篇论文")).toBe("已有内容 解释这篇论文");
    expect(mergeSpeechTranscript("已有内容 解释 后续输入", "解释", "解释这篇论文"))
      .toBe("已有内容 解释这篇论文 后续输入");
  });

  it("uses natural separators for English and CJK dictation", () => {
    expect(mergeSpeechTranscript("explain", "", "this paper")).toBe("explain this paper");
    expect(mergeSpeechTranscript("解释", "", "这篇论文")).toBe("解释这篇论文");
    expect(mergeSpeechTranscript("解释这篇论文", "", "？")).toBe("解释这篇论文？");
  });

  it("maps permission, microphone, and quiet-input failures", () => {
    expect(speechInputFailure("service-not-allowed")?.message).toContain("系统设置");
    expect(speechInputFailure("audio-capture")?.message).toContain("麦克风");
    expect(speechInputFailure("no-speech")?.level).toBe("warning");
    expect(speechInputFailure("aborted")).toBeUndefined();
  });
});
