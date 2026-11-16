/** @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { LlmConfiguration, LlmModelDescriptor } from "@research-assistant/core/ai";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/services/aiAssistanceGateway";
import { ReaderAgentModelControl } from "./ReaderAgentModelControl";

vi.mock("../../../../../../apps/web/src/core/features/models/llmModelCatalogCache", () => ({
  cacheLlmModels: vi.fn(),
  readCachedLlmModels: vi.fn(() => undefined),
}));

const models: readonly LlmModelDescriptor[] = [
  { id: "vision-default", displayName: "Vision Default", description: "", isDefault: true, supportedReasoningEfforts: [] },
];

const configuration: LlmConfiguration = {
  providers: [{
    providerId: "openai",
    displayName: "ChatGPT / Codex",
    authMode: "chatgpt-login",
    protocol: "codex-cli",
    modelKinds: ["text", "vision"],
    defaultBaseUrls: [],
    baseUrl: "",
    usesDefaultBaseUrl: true,
    enabled: true,
    timeoutMs: 60_000,
    credentialConfigured: true,
  }],
  defaultModels: [{ routeKind: "default", routeId: "vision", label: "视觉模型", modelKind: "vision", selection: { providerId: "openai", modelId: "vision-default" } }],
  useCases: [{ routeKind: "use_case", routeId: "paper_reading_chat", label: "文献阅读", modelKind: "vision", selection: { providerId: "openai", modelId: "vision-default" } }],
};

describe("ReaderAgentModelControl", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it("selects the vision route and preserves a controlled session model", async () => {
    const onSelection = vi.fn();
    const gateway = {
      getLlmConfiguration: vi.fn(async () => configuration),
      fetchLlmProviderModels: vi.fn(async () => ({ providerId: "openai" as const, models })),
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(<ReaderAgentModelControl disabled={false} gateway={gateway} onSelection={onSelection} selection={undefined} />));
    const trigger = container.querySelector<HTMLButtonElement>('[aria-label="选择 AI 文献阅读模型"]')!;
    expect(trigger.textContent).toBe("Vision Default");
    await act(async () => trigger.click());
    await act(async () => document.querySelector<HTMLButtonElement>('button[aria-label="选择模型"]')!.click());
    expect(document.querySelector('[aria-label="可用模型"]')).not.toBeNull();
    expect(document.querySelector('[aria-label="选择推理强度"]')).toBeNull();
    expect(onSelection).toHaveBeenCalledWith({ providerId: "openai", modelId: "vision-default" });
  });

  it("falls back when a controlled session model no longer matches the vision provider", async () => {
    const onSelection = vi.fn();
    const gateway = {
      getLlmConfiguration: vi.fn(async () => configuration),
      fetchLlmProviderModels: vi.fn(async () => ({ providerId: "openai" as const, models })),
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(<ReaderAgentModelControl
      disabled={false}
      gateway={gateway}
      onSelection={onSelection}
      selection={{ providerId: "openai", modelId: "qwen-vl" }}
    />));
    expect(onSelection).toHaveBeenCalledWith({ providerId: "openai", modelId: "vision-default" });
  });
});
