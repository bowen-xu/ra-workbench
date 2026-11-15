/** @vitest-environment happy-dom */

import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type {
  SpeechInputFailure,
  SpeechRecognitionErrorEventLike,
  SpeechRecognitionLike,
  SpeechRecognitionResultEventLike,
} from "./speechRecognition";
import { useSpeechInput } from "./useSpeechInput";

class FakeSpeechRecognition implements SpeechRecognitionLike {
  static latest: FakeSpeechRecognition | undefined;

  continuous = false;
  interimResults = false;
  lang = "";
  maxAlternatives = 0;
  onend: (() => void) | null = null;
  onerror: ((event: SpeechRecognitionErrorEventLike) => void) | null = null;
  onresult: ((event: SpeechRecognitionResultEventLike) => void) | null = null;
  onstart: (() => void) | null = null;
  abort = vi.fn();
  start = vi.fn(() => this.onstart?.());
  stop = vi.fn(() => this.onend?.());

  constructor() {
    FakeSpeechRecognition.latest = this;
  }

  result(transcript: string): void {
    this.onresult?.({
      resultIndex: 0,
      results: { 0: { 0: { transcript }, isFinal: false, length: 1 }, length: 1 },
    });
  }

  fail(error: string): void {
    this.onerror?.({ error });
  }
}

describe("useSpeechInput", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    Object.assign(window, { webkitSpeechRecognition: FakeSpeechRecognition });
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    Reflect.deleteProperty(window, "webkitSpeechRecognition");
    FakeSpeechRecognition.latest = undefined;
    container.remove();
  });

  function Harness({
    onFailure = () => undefined,
  }: {
    readonly onFailure?: (failure: SpeechInputFailure) => void;
  }): React.JSX.Element {
    const [value, setValue] = useState("已有内容");
    const speech = useSpeechInput({ disabled: false, onChange: setValue, onFailure, value });
    return <>
      <output>{value}</output>
      <span>{speech.state}</span>
      <button disabled={!speech.isSupported} onClick={speech.toggle} type="button">toggle</button>
    </>;
  }

  it("starts, streams replacement text, and stops cleanly", async () => {
    await act(async () => root.render(<Harness />));
    await act(async () => container.querySelector("button")?.click());
    const recognition = FakeSpeechRecognition.latest;
    expect(recognition?.start).toHaveBeenCalledOnce();
    expect(recognition?.continuous).toBe(true);
    expect(recognition?.interimResults).toBe(true);
    expect(recognition?.lang).toBe("zh-CN");
    expect(container.textContent).toContain("listening");

    await act(async () => recognition?.result("解释"));
    expect(container.querySelector("output")?.textContent).toBe("已有内容解释");
    await act(async () => recognition?.result("解释这篇论文"));
    expect(container.querySelector("output")?.textContent).toBe("已有内容解释这篇论文");

    await act(async () => container.querySelector("button")?.click());
    expect(recognition?.stop).toHaveBeenCalledOnce();
    expect(container.textContent).toContain("idle");
  });

  it("reports permission errors and aborts the active recognizer", async () => {
    const onFailure = vi.fn<(failure: SpeechInputFailure) => void>();
    await act(async () => root.render(<Harness onFailure={onFailure} />));
    await act(async () => container.querySelector("button")?.click());
    const recognition = FakeSpeechRecognition.latest;
    await act(async () => recognition?.fail("not-allowed"));
    expect(recognition?.abort).toHaveBeenCalledOnce();
    expect(onFailure).toHaveBeenCalledWith(expect.objectContaining({ level: "error" }));
    expect(container.textContent).toContain("idle");
  });
});
