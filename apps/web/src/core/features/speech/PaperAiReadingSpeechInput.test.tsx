/** @vitest-environment happy-dom */

import { act, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { SpeechInputAdapterProvider } from "./SpeechInputAdapter";
import { writingDemoSpeechAdapter } from "../../../app/previews/agent/WritingDemoSpeechAdapter";

import type {
  SpeechInputState,
  SpeechRecognitionErrorEventLike,
  SpeechRecognitionLike,
  SpeechRecognitionResultEventLike,
} from "./speechRecognition";
import {
  PaperAiReadingSpeechInput,
  speechInputHostFailure,
} from "./PaperAiReadingSpeechInput";

const notifications = vi.hoisted(() => ({ error: vi.fn(), warning: vi.fn() }));
const tauri = vi.hoisted(() => ({ isTauri: vi.fn(() => false) }));
vi.mock("../../services/notifications", () => ({ notify: notifications }));
vi.mock("@tauri-apps/api/core", () => ({ isTauri: tauri.isTauri }));

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
}

describe("Paper AI reading speech input", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    notifications.error.mockReset();
    notifications.warning.mockReset();
    tauri.isTauri.mockReset();
    tauri.isTauri.mockReturnValue(false);
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    Reflect.deleteProperty(window, "webkitSpeechRecognition");
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
    Reflect.deleteProperty(window, "confirm");
    vi.restoreAllMocks();
    FakeSpeechRecognition.latest = undefined;
    container.remove();
  });

  function Harness({ onStateChange }: { readonly onStateChange?: (state: SpeechInputState) => void }): React.JSX.Element {
    const [value, setValue] = useState("");
    return <>
      <output>{value}</output>
      <PaperAiReadingSpeechInput
        disabled={false}
        onChange={setValue}
        value={value}
        {...(onStateChange === undefined ? {} : { onStateChange })}
      />
    </>;
  }

  it("disables the microphone when the runtime has no recognition constructor", async () => {
    await act(async () => root.render(<Harness />));
    const button = container.querySelector("button");
    expect(button?.disabled).toBe(true);
    expect(button?.getAttribute("aria-label")).toBe("当前环境不支持语音输入");
  });

  it("blocks Web Speech before start in an unbundled Tauri development process", async () => {
    tauri.isTauri.mockReturnValue(true);
    vi.spyOn(window.navigator, "platform", "get").mockReturnValue("MacIntel");
    Object.assign(window, {
      __TAURI_INTERNALS__: {},
      webkitSpeechRecognition: FakeSpeechRecognition,
    });
    await act(async () => root.render(<Harness />));

    const button = container.querySelector("button");
    expect(speechInputHostFailure({
      desktop: true,
      hostname: "127.0.0.1",
      platform: "MacIntel",
      protocol: "http:",
      tauriInternals: true,
    })).toContain("裸进程");
    expect(speechInputHostFailure({
      desktop: true,
      hostname: "localhost",
      platform: "MacIntel",
      protocol: "tauri:",
      tauriInternals: true,
    })).toBeUndefined();
    expect(speechInputHostFailure({
      desktop: true,
      hostname: "localhost",
      platform: "Win32",
      protocol: "http:",
      tauriInternals: true,
    })).toBeUndefined();
    expect(button?.disabled).toBe(false);
    expect(button?.getAttribute("aria-label")).toContain("pnpm bundle:dev");
    await act(async () => button?.click());
    expect(FakeSpeechRecognition.latest).toBeUndefined();
    expect(notifications.warning).toHaveBeenCalledWith(expect.stringContaining("pnpm bundle:dev"));
  });

  it("asks for disclosure consent, streams text, and exposes active state", async () => {
    Object.assign(window, { webkitSpeechRecognition: FakeSpeechRecognition });
    const confirm = vi.fn(() => true);
    Object.assign(window, { confirm });
    const onStateChange = vi.fn<(state: SpeechInputState) => void>();
    await act(async () => root.render(<Harness onStateChange={onStateChange} />));
    const button = container.querySelector("button");
    await act(async () => button?.click());
    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("可能使用联网识别"));
    expect(button?.getAttribute("aria-label")).toBe("停止输入");
    expect(button?.getAttribute("aria-pressed")).toBe("true");

    await act(async () => FakeSpeechRecognition.latest?.result("解释这篇论文"));
    expect(container.querySelector("output")?.textContent).toBe("解释这篇论文");
    expect(onStateChange).toHaveBeenCalledWith("listening");

    await act(async () => button?.click());
    expect(FakeSpeechRecognition.latest?.stop).toHaveBeenCalledOnce();
    expect(button?.getAttribute("aria-pressed")).toBe("false");
  });

  it("does not start recognition when the disclosure is declined", async () => {
    Object.assign(window, { webkitSpeechRecognition: FakeSpeechRecognition });
    Object.assign(window, { confirm: vi.fn(() => false) });
    await act(async () => root.render(<Harness />));
    await act(async () => container.querySelector("button")?.click());
    expect(FakeSpeechRecognition.latest).toBeUndefined();
  });

  it("uses the injected mock without browser recognition or permission prompts", async () => {
    vi.useFakeTimers();
    try {
      const confirm = vi.fn(() => false);
      Object.assign(window, { confirm, webkitSpeechRecognition: FakeSpeechRecognition });
      await act(async () => root.render(<SpeechInputAdapterProvider adapter={writingDemoSpeechAdapter}><Harness /></SpeechInputAdapterProvider>));
      const button = container.querySelector<HTMLButtonElement>("button")!;
      expect(button.getAttribute("aria-label")).toBe("语音输入");
      await act(async () => button.click());
      expect(button.getAttribute("aria-label")).toBe("停止输入");
      await act(async () => vi.advanceTimersByTime(400));
      expect(container.querySelector("output")?.textContent).toBe("模拟语音输入：请帮我");
      await act(async () => vi.advanceTimersByTime(500));
      expect(container.querySelector("output")?.textContent).toBe("模拟语音输入：请帮我完善摘要部分。");
      await act(async () => button.click());
      expect(button.getAttribute("aria-label")).toBe("语音输入");
      expect(confirm).not.toHaveBeenCalled();
      expect(FakeSpeechRecognition.latest).toBeUndefined();
      expect(notifications.error).not.toHaveBeenCalled();
      await act(async () => button.click());
      await act(async () => button.click());
      await act(async () => vi.advanceTimersByTime(1000));
      expect(container.querySelector("output")?.textContent).toBe("模拟语音输入：请帮我完善摘要部分。");
    } finally { vi.useRealTimers(); }
  });
});
