/** @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { LlmConfiguration, LlmModelDescriptor } from "@research-assistant/core/ai";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AiAssistanceGateway } from "../../../services/aiAssistanceGateway";
import { WritingAgentModelControl } from "./WritingAgentModelControl";

vi.mock("../../models/llmModelCatalogCache", () => ({
  cacheLlmModels: vi.fn(),
  readCachedLlmModels: vi.fn(() => undefined),
}));

const models: readonly LlmModelDescriptor[] = [
  { id: "gpt-5.6", displayName: "GPT-5.6", description: "", supportsFastMode: true, isDefault: true, defaultReasoningEffort: "medium", supportedReasoningEfforts: ["low", "medium", "high"] },
  { id: "gpt-5.5", displayName: "GPT-5.5", description: "", supportsFastMode: true, isDefault: false, defaultReasoningEffort: "medium", supportedReasoningEfforts: ["low", "medium", "high"] },
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
  defaultModels: [{ routeKind: "default", routeId: "text", label: "文本模型", modelKind: "text", selection: { providerId: "openai", modelId: "gpt-5.6", reasoningEffort: "medium" } }],
  useCases: [],
};

describe("WritingAgentModelControl", () => {
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

  it("loads the configured text model catalog and keeps selection local to the composer", async () => {
    const onSelection = vi.fn();
    const gateway = {
      getLlmConfiguration: vi.fn(async () => configuration),
      fetchLlmProviderModels: vi.fn(async () => ({ providerId: "openai" as const, models })),
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(<WritingAgentModelControl disabled={false} gateway={gateway} onSelection={onSelection} />));
    const trigger = container.querySelector<HTMLButtonElement>('[aria-label="选择 AI 写作模型"]')!;
    expect(trigger.textContent).toContain("GPT-5.6Medium");
    expect(trigger.getAttribute("title")).toBeNull();
    expect(container.querySelector("select")).toBeNull();
    await act(async () => trigger.click());
    expect(document.querySelector('[aria-label="模型和推理强度"]')).not.toBeNull();
    await act(async () => document.querySelector<HTMLButtonElement>('[aria-label="选择模型"]')!.click());
    const options = document.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]');
    expect(options[0]?.getAttribute("aria-checked")).toBe("true");

    await act(async () => {
      options[1]!.click();
    });
    expect(onSelection).toHaveBeenLastCalledWith({ providerId: "openai", modelId: "gpt-5.5", reasoningEffort: "medium" });

    expect(document.querySelector('[aria-label="可用模型"]')).toBeNull();
    await act(async () => trigger.click());
    const effort = document.querySelector<HTMLInputElement>('[aria-label="选择推理强度"]')!;
    await act(async () => {
      effort.value = "2";
      effort.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(onSelection).toHaveBeenLastCalledWith({ providerId: "openai", modelId: "gpt-5.5", reasoningEffort: "high" });
  });

  it("refreshes after shared model configuration changes", async () => {
    const onSelection = vi.fn();
    const refreshedModels: readonly LlmModelDescriptor[] = [
      { id: "gpt-5.7", displayName: "GPT-5.7", description: "", supportsFastMode: true, isDefault: true, defaultReasoningEffort: "high", supportedReasoningEfforts: ["low", "medium", "high"] },
    ];
    const updated: LlmConfiguration = {
      ...configuration,
      defaultModels: [{ routeKind: "default", routeId: "text", label: "文本模型", modelKind: "text", selection: { providerId: "openai", modelId: "gpt-5.7", reasoningEffort: "high" } }],
    };
    const gateway = {
      getLlmConfiguration: vi.fn()
        .mockResolvedValueOnce(configuration)
        .mockResolvedValueOnce(updated),
      fetchLlmProviderModels: vi.fn()
        .mockResolvedValueOnce({ providerId: "openai" as const, models })
        .mockResolvedValueOnce({ providerId: "openai" as const, models: refreshedModels }),
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(<WritingAgentModelControl disabled={false} gateway={gateway} onSelection={onSelection} />));
    await act(async () => window.dispatchEvent(new Event("research-assistant:llm-configuration-changed")));
    expect(container.querySelector(".agent-model-picker-trigger")?.textContent).toBe("GPT-5.7High");
    expect(onSelection).toHaveBeenLastCalledWith({ providerId: "openai", modelId: "gpt-5.7", reasoningEffort: "high" });
  });
  it.each(["fast", "default", undefined] as const)("resets model and effort while preserving service tier %s", async (serviceTier) => {
    const onSelection = vi.fn();
    const gateway = {
      getLlmConfiguration: vi.fn(async () => configuration),
      fetchLlmProviderModels: vi.fn(async () => ({ providerId: "openai" as const, models })),
    };
    await act(async () => root.render(<WritingAgentModelControl disabled={false} gateway={gateway} onSelection={onSelection}
      selection={{ providerId: "openai", modelId: "gpt-5.5", reasoningEffort: "medium", ...(serviceTier === undefined ? {} : { serviceTier }) }} />));
    await act(async () => container.querySelector<HTMLButtonElement>("button")!.click());
    await act(async () => document.querySelector<HTMLButtonElement>('[aria-label="恢复默认"]')!.click());
    const resetSelection = { providerId: "openai" as const, modelId: "gpt-5.6", reasoningEffort: "medium" as const,
      ...(serviceTier === undefined ? {} : { serviceTier }) };
    expect(onSelection).toHaveBeenLastCalledWith(resetSelection);
    await act(async () => root.render(<WritingAgentModelControl disabled={false} gateway={gateway} onSelection={onSelection} selection={resetSelection} />));
    expect(document.querySelector<HTMLButtonElement>('[aria-label="恢复默认"]')?.disabled).toBe(true);
    expect(document.querySelector('[aria-label="Fast mode"]')?.getAttribute("aria-pressed")).toBe(String(serviceTier === "fast"));
    await act(async () => document.querySelector<HTMLButtonElement>('[aria-label="选择模型"]')!.click());
    await act(async () => document.querySelector<HTMLButtonElement>('.agent-model-picker-default')!.click());
    expect(onSelection).toHaveBeenLastCalledWith({ providerId: "openai", modelId: "gpt-5.6", reasoningEffort: "medium" });
    expect(document.querySelector('.agent-model-picker-popover')).toBeNull();
  });

  it("keeps disabled or unconfigured controls closed", async () => {
    const gateway = {
      getLlmConfiguration: vi.fn(async () => ({ providers: [], defaultModels: [], useCases: [] })),
      fetchLlmProviderModels: vi.fn(),
    };
    await act(async () => root.render(<WritingAgentModelControl disabled={false} gateway={gateway} onSelection={vi.fn()} />));
    const trigger = container.querySelector<HTMLButtonElement>("button")!;
    expect(trigger.disabled).toBe(true);
    await act(async () => trigger.click());
    expect(document.querySelector('.agent-model-picker-popover')).toBeNull();
  });
  it("toggles Fast mode independently of effort and closes only the model layer on Escape", async () => {
    const onSelection = vi.fn();
    const gateway = { getLlmConfiguration: vi.fn(async () => configuration), fetchLlmProviderModels: vi.fn(async () => ({ providerId: "openai" as const, models })) };
    await act(async () => root.render(<WritingAgentModelControl disabled={false} gateway={gateway} onSelection={onSelection} />));
    const trigger = container.querySelector<HTMLButtonElement>("button")!;
    await act(async () => trigger.click());
    const fast = document.querySelector<HTMLButtonElement>('[aria-label="Fast mode"]')!;
    expect(fast.disabled).toBe(false);
    await act(async () => fast.click());
    expect(onSelection).toHaveBeenLastCalledWith({ providerId: "openai", modelId: "gpt-5.6", reasoningEffort: "medium", serviceTier: "fast" });
    expect(fast.getAttribute("aria-pressed")).toBe("true");
    await act(async () => fast.click());
    expect(onSelection).toHaveBeenLastCalledWith({ providerId: "openai", modelId: "gpt-5.6", reasoningEffort: "medium", serviceTier: "default" });
    await act(async () => document.querySelector<HTMLButtonElement>('button[aria-label="选择模型"]')!.click());
    await act(async () => document.querySelector('[role="menuitemradio"]')!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    expect(document.querySelector('[aria-label="可用模型"]')).toBeNull();
    expect(document.querySelector('[aria-label="模型和推理强度"]')).not.toBeNull();
  });
  it("opens the model list directly for other providers", async () => {
    const onSelection = vi.fn();
    const other: LlmConfiguration = { ...configuration, providers: configuration.providers.map(provider => ({ ...provider, providerId: "deepseek", authMode: "api-key", protocol: "openai-compatible" })),
      defaultModels: [{ ...configuration.defaultModels[0]!, selection: { providerId: "deepseek", modelId: "deepseek-v4-pro" } }] };
    const gateway = { getLlmConfiguration: vi.fn(async () => other), fetchLlmProviderModels: vi.fn(async () => ({ providerId: "deepseek" as const, models: [{ id: "deepseek-v4-pro", displayName: "DeepSeek", description: "", isDefault: true, supportedReasoningEfforts: [] }] })) };
    await act(async () => root.render(<WritingAgentModelControl disabled={false} gateway={gateway} onSelection={onSelection} />));
    await act(async () => container.querySelector<HTMLButtonElement>("button")!.click());
    expect(document.querySelector('[aria-label="可用模型"]')).not.toBeNull();
    expect(document.querySelector('[aria-label="Fast mode"]')).toBeNull();
  });

});
