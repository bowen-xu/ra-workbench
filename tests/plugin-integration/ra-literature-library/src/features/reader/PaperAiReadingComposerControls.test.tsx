/** @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { PaperAiReadingTurn } from "@research-assistant/core/ai";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { PaperAiReadingContextIndicator } from "./PaperAiReadingComposerControls";

function turn(inputTokens: number, contextWindowTokens?: number): PaperAiReadingTurn {
  return {
    id: "turn",
    sessionId: "session",
    ordinal: 0,
    userMessage: "Question",
    assistantMessage: "Answer",
    messages: [
      { id: "message-user", role: "user", content: "Question" },
      { id: "message-assistant", role: "assistant", content: "Answer" },
    ],
    providerId: "openai",
    modelId: "gpt-5.5",
    reasoningEffort: "high",
    status: "succeeded",
    inputTokens,
    outputTokens: 100,
    conceptRetrievalEnabled: false,
    ...(contextWindowTokens === undefined ? {} : {
      contextWindowTokens,
      contextWindowSource: "provider_runtime" as const,
    }),
    createdAt: "now",
    finishedAt: "later",
  };
}

describe("Paper AI context indicator", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  async function toggle(): Promise<void> {
    const trigger = container.querySelector<HTMLButtonElement>('[aria-label="查看上下文窗口用量"]');
    if (trigger === null) throw new Error("missing context trigger");
    await act(async () => trigger.click());
  }

  it("distinguishes missing usage, unknown limits, and the 95 percent danger state", async () => {
    await act(async () => root.render(<PaperAiReadingContextIndicator turn={undefined} />));
    await toggle();
    expect(document.body.textContent).toContain("暂无 provider 返回的真实用量");
    await toggle();

    await act(async () => root.render(<PaperAiReadingContextIndicator turn={turn(4_096)} />));
    await toggle();
    expect(document.body.textContent).toContain("上一轮输入：4,096 tokens");
    expect(document.body.textContent).toContain("上下文上限未知");
    await toggle();

    await act(async () => root.render(<PaperAiReadingContextIndicator turn={turn(95_000, 100_000)} />));
    expect(container.querySelector(".paper-ai-context-trigger")?.classList.contains("danger")).toBe(true);
    await toggle();
    expect(document.body.textContent).toContain("已用 95%");
    expect(document.body.textContent).toContain("剩余 5,000 tokens");
  });
});
