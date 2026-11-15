import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useSpeechInputAdapter } from "./SpeechInputAdapter";

import {
  getSpeechRecognitionConstructor,
  mergeSpeechTranscript,
  speechInputFailure,
  speechRecognitionTranscript,
  type SpeechInputFailure,
  type SpeechInputState,
  type SpeechRecognitionLike,
} from "./speechRecognition";

interface UseSpeechInputOptions {
  readonly disabled: boolean;
  readonly onChange: (value: string) => void;
  readonly onFailure: (failure: SpeechInputFailure) => void;
  readonly value: string;
}

interface SpeechInputControl {
  readonly isSupported: boolean;
  readonly state: SpeechInputState;
  readonly toggle: () => void;
}

function releaseRecognition(recognition: SpeechRecognitionLike): void {
  recognition.onstart = null;
  recognition.onresult = null;
  recognition.onerror = null;
  recognition.onend = null;
}

export function useSpeechInput({
  disabled,
  onChange,
  onFailure,
  value,
}: UseSpeechInputOptions): SpeechInputControl {
  const adapter = useSpeechInputAdapter();
  const constructor = useMemo(
    () => adapter?.recognition ?? (typeof window === "undefined" ? undefined : getSpeechRecognitionConstructor(window)),
    [adapter],
  );
  const [state, setState] = useState<SpeechInputState>("idle");
  const recognitionRef = useRef<SpeechRecognitionLike | undefined>(undefined);
  const previousSpeechRef = useRef("");
  const valueRef = useRef(value);
  const onChangeRef = useRef(onChange);
  const onFailureRef = useRef(onFailure);

  useEffect(() => { valueRef.current = value; }, [value]);
  useEffect(() => { onChangeRef.current = onChange; }, [onChange]);
  useEffect(() => { onFailureRef.current = onFailure; }, [onFailure]);

  const finish = useCallback((recognition: SpeechRecognitionLike, shouldAbort: boolean): void => {
    if (recognitionRef.current !== recognition) return;
    recognitionRef.current = undefined;
    previousSpeechRef.current = "";
    releaseRecognition(recognition);
    if (shouldAbort) {
      try { recognition.abort(); } catch { /* The engine has already stopped. */ }
    }
    setState("idle");
  }, []);

  const toggle = useCallback((): void => {
    const active = recognitionRef.current;
    if (active !== undefined) {
      setState("stopping");
      try {
        active.stop();
      } catch {
        finish(active, true);
        onFailureRef.current({ level: "error", message: "无法停止语音输入，请稍后重试。" });
      }
      return;
    }
    if (constructor === undefined || disabled) return;

    const recognition = new constructor();
    recognition.continuous = true;
    recognition.interimResults = true;
    recognition.maxAlternatives = 1;
    recognition.lang = "zh-CN";
    previousSpeechRef.current = "";
    recognitionRef.current = recognition;
    recognition.onstart = () => {
      if (recognitionRef.current === recognition) setState("listening");
    };
    recognition.onresult = (event) => {
      if (recognitionRef.current !== recognition) return;
      const nextSpeech = speechRecognitionTranscript(event);
      const nextValue = mergeSpeechTranscript(valueRef.current, previousSpeechRef.current, nextSpeech);
      previousSpeechRef.current = nextSpeech;
      valueRef.current = nextValue;
      onChangeRef.current(nextValue);
    };
    recognition.onerror = (event) => {
      const failure = speechInputFailure(event.error);
      finish(recognition, true);
      if (failure !== undefined) onFailureRef.current(failure);
    };
    recognition.onend = () => finish(recognition, false);
    setState("starting");
    try {
      recognition.start();
    } catch {
      finish(recognition, true);
      onFailureRef.current({ level: "error", message: "无法启动语音输入，请稍后重试。" });
    }
  }, [constructor, disabled, finish]);

  useEffect(() => {
    if (!disabled || recognitionRef.current === undefined) return;
    finish(recognitionRef.current, true);
  }, [disabled, finish]);

  useEffect(() => () => {
    const recognition = recognitionRef.current;
    if (recognition === undefined) return;
    recognitionRef.current = undefined;
    releaseRecognition(recognition);
    try { recognition.abort(); } catch { /* The engine has already stopped. */ }
  }, [constructor]);

  return {
    isSupported: constructor !== undefined,
    state,
    toggle,
  };
}
