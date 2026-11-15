// @vitest-environment happy-dom

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { LlmConfiguration, LlmModelRouteDraft, LlmModelSelection } from "@research-assistant/core/ai";

import type { AiAssistanceGateway } from "../../services/aiAssistanceGateway";
import { cacheLlmModels } from "./llmModelCatalogCache";
import { LlmRoutePicker } from "./LlmRoutePicker";

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean })
  .IS_REACT_ACT_ENVIRONMENT = true;

const configuration: LlmConfiguration = {
  providers: [{
    providerId: "qwen", displayName: "Qwen", authMode: "api-key",
    protocol: "openai-compatible", modelKinds: ["vision"],
    defaultBaseUrls: ["https://dashscope.aliyuncs.com/compatible-mode/v1"], baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    usesDefaultBaseUrl: true, enabled: true, timeoutMs: 60_000,
    credentialConfigured: true,
  }],
  defaultModels: [{
    routeKind: "default", routeId: "vision", label: "视觉模型", modelKind: "vision",
    selection: { providerId: "qwen", modelId: "model-a" },
  }],
  useCases: [{
    routeKind: "use_case", routeId: "paper_overview", label: "生成论文概览", modelKind: "vision",
  }],
};

function installMemoryLocalStorage(): void {
  const values = new Map<string, string>();
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    value: {
      clear: () => values.clear(),
      getItem: (key: string) => values.get(key) ?? null,
      key: (index: number) => [...values.keys()][index] ?? null,
      get length() { return values.size; },
      removeItem: (key: string) => values.delete(key),
      setItem: (key: string, value: string) => values.set(key, value),
    } satisfies Storage,
  });
}

describe("local LLM route picker", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    installMemoryLocalStorage();
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it("persists edits to the shared use-case route unless only-this-run is selected", async () => {
    let current = configuration;
    const saveLlmModelRoutes = vi.fn(async (drafts: readonly LlmModelRouteDraft[]) => {
      current = {
        ...configuration,
        useCases: [{ ...configuration.useCases[0]!, selection: drafts[0]?.selection }],
      } as LlmConfiguration;
      return current;
    });
    const gateway = {
      getLlmConfiguration: vi.fn(async () => current),
      fetchLlmProviderModels: vi.fn(async () => ({
        providerId: "qwen" as const,
        models: ["model-a", "model-b"].map((id) => ({
          id, displayName: id, description: "", isDefault: id === "model-a",
          supportedReasoningEfforts: [],
        })),
      })),
      saveLlmModelRoutes,
    } as unknown as AiAssistanceGateway;
    const selections: Array<LlmModelSelection | undefined> = [];
    await act(async () => root.render(<LlmRoutePicker gateway={gateway} onSelection={(value) => selections.push(value)} taskKind="paper_overview" />));
    await act(async () => undefined);

    const provider = container.querySelector<HTMLSelectElement>("select")!;
    await act(async () => {
      provider.dispatchEvent(new Event("change", { bubbles: true }));
    });
    expect(saveLlmModelRoutes).toHaveBeenCalledWith([{
      routeKind: "use_case", routeId: "paper_overview",
      selection: { providerId: "qwen", modelId: "model-a" },
    }]);

    const onlyRun = container.querySelector<HTMLInputElement>('.llm-route-picker-run input')!;
    await act(async () => { onlyRun.click(); });
    expect(selections.at(-1)).toEqual({ providerId: "qwen", modelId: "model-a" });
  });

  it("shows the concrete route label and optional description", async () => {
    const gateway = {
      getLlmConfiguration: vi.fn(async () => configuration),
      fetchLlmProviderModels: vi.fn(async () => ({
        providerId: "qwen" as const,
        models: [{
          id: "model-a", displayName: "model-a", description: "", isDefault: true,
          supportedReasoningEfforts: [],
        }],
      })),
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(
      <LlmRoutePicker
        description="用于从结构化上下文提取候选。"
        gateway={gateway}
        legend="提取结构化信息模型"
        onSelection={() => undefined}
        taskKind="paper_overview"
      />,
    ));
    await act(async () => undefined);

    expect(container.querySelector("legend")?.textContent).toBe("提取结构化信息模型");
    expect(container.textContent).toContain("用于从结构化上下文提取候选。");
    expect(container.textContent).toContain("模型服务");
    expect(container.textContent).toContain("修改后会保存为“提取结构化信息模型”的功能默认模型。");
  });

  it("repairs a saved model id that does not belong to the selected provider catalog", async () => {
    let current: LlmConfiguration = {
      ...configuration,
      providers: [{
        providerId: "openai", displayName: "ChatGPT / Codex", authMode: "chatgpt-login",
        protocol: "codex-cli", modelKinds: ["vision"], defaultBaseUrls: [], baseUrl: "",
        usesDefaultBaseUrl: true, enabled: true, timeoutMs: 120_000, credentialConfigured: false,
      }],
      useCases: [{
        routeKind: "use_case", routeId: "paper_overview", label: "生成论文概览", modelKind: "vision",
        selection: { providerId: "openai", modelId: "qwen3-vl-flash-2026-01-22" },
      }],
    };
    const saveLlmModelRoutes = vi.fn(async (drafts: readonly LlmModelRouteDraft[]) => {
      current = {
        ...current,
        useCases: [{ ...current.useCases[0]!, selection: drafts[0]?.selection }],
      } as LlmConfiguration;
      return current;
    });
    const gateway = {
      getLlmConfiguration: vi.fn(async () => current),
      fetchLlmProviderModels: vi.fn(async () => ({
        providerId: "openai" as const,
        models: [{
          id: "gpt-5.6-terra", displayName: "GPT-5.6-Terra", description: "", isDefault: true,
          defaultReasoningEffort: "medium" as const,
          supportedReasoningEfforts: ["medium"] as const,
        }],
      })),
      saveLlmModelRoutes,
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(<LlmRoutePicker gateway={gateway} onSelection={() => undefined} taskKind="paper_overview" />));
    await act(async () => undefined);

    expect(saveLlmModelRoutes).toHaveBeenCalledWith([{
      routeKind: "use_case",
      routeId: "paper_overview",
      selection: { providerId: "openai", modelId: "gpt-5.6-terra", reasoningEffort: "medium" },
    }]);
    expect(container.querySelector<HTMLInputElement>('input[list="local-models-paper_overview"]')?.value)
      .toBe("gpt-5.6-terra");
  });

  it("lists disabled vision providers with a status instead of hiding them", async () => {
    const current: LlmConfiguration = {
      ...configuration,
      providers: [
        {
          providerId: "openai", displayName: "ChatGPT / Codex", authMode: "chatgpt-login",
          protocol: "codex-cli", modelKinds: ["vision"], defaultBaseUrls: [], baseUrl: "",
          usesDefaultBaseUrl: true, enabled: true, timeoutMs: 120_000, credentialConfigured: false,
        },
        {
          providerId: "qwen", displayName: "Qwen", authMode: "api-key", protocol: "openai-compatible",
          modelKinds: ["text", "vision"], defaultBaseUrls: ["https://dashscope.aliyuncs.com/compatible-mode/v1"],
          baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1", usesDefaultBaseUrl: true,
          enabled: false, timeoutMs: 60_000, credentialConfigured: true,
        },
      ],
      defaultModels: [{
        routeKind: "default", routeId: "vision", label: "视觉模型", modelKind: "vision",
        selection: { providerId: "openai", modelId: "gpt-5.6-terra", reasoningEffort: "medium" },
      }],
    };
    const gateway = {
      getLlmConfiguration: vi.fn(async () => current),
      fetchLlmProviderModels: vi.fn(async () => ({
        providerId: "openai" as const,
        models: [{
          id: "gpt-5.6-terra", displayName: "GPT-5.6-Terra", description: "", isDefault: true,
          defaultReasoningEffort: "medium" as const,
          supportedReasoningEfforts: ["medium"] as const,
        }],
      })),
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(<LlmRoutePicker gateway={gateway} onSelection={() => undefined} taskKind="paper_overview" />));
    await act(async () => undefined);

    const options = [...container.querySelectorAll<HTMLOptionElement>("select option")];
    expect(options.map((option) => option.textContent)).toContain("Qwen（未启用）");
    expect(options.find((option) => option.textContent === "Qwen（未启用）")?.disabled).toBe(true);
  });

  it("falls back from a disabled saved vision provider to an enabled vision provider", async () => {
    let current: LlmConfiguration = {
      ...configuration,
      providers: [
        {
          providerId: "openai", displayName: "ChatGPT / Codex", authMode: "chatgpt-login",
          protocol: "codex-cli", modelKinds: ["text", "vision"], defaultBaseUrls: [], baseUrl: "",
          usesDefaultBaseUrl: true, enabled: true, timeoutMs: 120_000, credentialConfigured: false,
        },
        {
          providerId: "qwen", displayName: "Qwen", authMode: "api-key", protocol: "openai-compatible",
          modelKinds: ["text", "vision"], defaultBaseUrls: ["https://dashscope.aliyuncs.com/compatible-mode/v1"],
          baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1", usesDefaultBaseUrl: true,
          enabled: false, timeoutMs: 60_000, credentialConfigured: true,
        },
      ],
      defaultModels: [],
      useCases: [{
        routeKind: "use_case", routeId: "paper_overview", label: "生成论文概览", modelKind: "vision",
        selection: { providerId: "qwen", modelId: "qwen3-vl-flash-2026-01-22" },
      }],
    };
    const saveLlmModelRoutes = vi.fn(async (drafts: readonly LlmModelRouteDraft[]) => {
      current = {
        ...current,
        useCases: [{ ...current.useCases[0]!, selection: drafts[0]?.selection }],
      } as LlmConfiguration;
      return current;
    });
    const gateway = {
      getLlmConfiguration: vi.fn(async () => current),
      fetchLlmProviderModels: vi.fn(async () => ({
        providerId: "openai" as const,
        models: [{
          id: "gpt-5.6-sol", displayName: "GPT-5.6-Sol", description: "", isDefault: true,
          defaultReasoningEffort: "medium" as const,
          supportedReasoningEfforts: ["medium"] as const,
        }],
      })),
      saveLlmModelRoutes,
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(<LlmRoutePicker gateway={gateway} onSelection={() => undefined} taskKind="paper_overview" />));
    await act(async () => undefined);

    expect(saveLlmModelRoutes).toHaveBeenCalledWith([{
      routeKind: "use_case",
      routeId: "paper_overview",
      selection: { providerId: "openai", modelId: "gpt-5.6-sol", reasoningEffort: "medium" },
    }]);
    expect(container.querySelector<HTMLSelectElement>("select")?.value).toBe("openai");
    expect(container.querySelector<HTMLInputElement>('input[list="local-models-paper_overview"]')?.value)
      .toBe("gpt-5.6-sol");
  });

  it("reselects an enabled provider after inheriting from an unavailable global vision route", async () => {
    let current: LlmConfiguration = {
      ...configuration,
      providers: [
        {
          providerId: "openai", displayName: "ChatGPT / Codex", authMode: "chatgpt-login",
          protocol: "codex-cli", modelKinds: ["text", "vision"], defaultBaseUrls: [], baseUrl: "",
          usesDefaultBaseUrl: true, enabled: true, timeoutMs: 120_000, credentialConfigured: false,
        },
        {
          providerId: "qwen", displayName: "Qwen", authMode: "api-key", protocol: "openai-compatible",
          modelKinds: ["text", "vision"], defaultBaseUrls: ["https://dashscope.aliyuncs.com/compatible-mode/v1"],
          baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1", usesDefaultBaseUrl: true,
          enabled: false, timeoutMs: 60_000, credentialConfigured: true,
        },
      ],
      defaultModels: [{
        routeKind: "default", routeId: "vision", label: "视觉模型", modelKind: "vision",
        selection: { providerId: "qwen", modelId: "qwen3-vl-flash-2026-01-22" },
      }],
      useCases: [{
        routeKind: "use_case", routeId: "paper_overview", label: "生成论文概览", modelKind: "vision",
        selection: { providerId: "openai", modelId: "gpt-5.6-sol", reasoningEffort: "medium" },
      }],
    };
    const saveLlmModelRoutes = vi.fn(async (drafts: readonly LlmModelRouteDraft[]) => {
      current = {
        ...current,
        useCases: [{ ...current.useCases[0]!, selection: drafts[0]?.selection }],
      } as LlmConfiguration;
      return current;
    });
    const gateway = {
      getLlmConfiguration: vi.fn(async () => current),
      fetchLlmProviderModels: vi.fn(async () => ({
        providerId: "openai" as const,
        models: [{
          id: "gpt-5.6-sol", displayName: "GPT-5.6-Sol", description: "", isDefault: true,
          defaultReasoningEffort: "medium" as const,
          supportedReasoningEfforts: ["medium"] as const,
        }],
      })),
      saveLlmModelRoutes,
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(<LlmRoutePicker gateway={gateway} onSelection={() => undefined} taskKind="paper_overview" />));
    await act(async () => undefined);
    const inherit = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent === "恢复继承全局视觉模型")!;

    await act(async () => { inherit.click(); });
    await act(async () => undefined);

    expect(saveLlmModelRoutes).toHaveBeenNthCalledWith(1, [{
      routeKind: "use_case",
      routeId: "paper_overview",
    }]);
    expect(saveLlmModelRoutes).toHaveBeenNthCalledWith(2, [{
      routeKind: "use_case",
      routeId: "paper_overview",
      selection: { providerId: "openai", modelId: "gpt-5.6-sol", reasoningEffort: "medium" },
    }]);
    expect(container.querySelector<HTMLSelectElement>("select")?.value).toBe("openai");
    expect(container.querySelector<HTMLInputElement>('input[list="local-models-paper_overview"]')?.value)
      .toBe("gpt-5.6-sol");
  });

  it("keeps an inherited Qwen model even when the provider catalog has a different default", async () => {
    let current: LlmConfiguration = {
      ...configuration,
      providers: [{
        providerId: "qwen", displayName: "Qwen", authMode: "api-key",
        protocol: "openai-compatible", modelKinds: ["text", "vision"],
        defaultBaseUrls: ["https://dashscope.aliyuncs.com/compatible-mode/v1"], baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        usesDefaultBaseUrl: true, enabled: true, timeoutMs: 60_000,
        credentialConfigured: true,
      }],
      defaultModels: [{
        routeKind: "default", routeId: "vision", label: "视觉模型", modelKind: "vision",
        selection: { providerId: "qwen", modelId: "qwen3-vl-flash-2026-01-22" },
      }],
      useCases: [{
        routeKind: "use_case", routeId: "paper_overview", label: "生成论文概览", modelKind: "vision",
        selection: { providerId: "openai", modelId: "gpt-5.6-sol", reasoningEffort: "medium" },
      }],
    };
    const saveLlmModelRoutes = vi.fn(async (drafts: readonly LlmModelRouteDraft[]) => {
      current = {
        ...current,
        useCases: [{ ...current.useCases[0]!, selection: drafts[0]?.selection }],
      } as LlmConfiguration;
      return current;
    });
    const gateway = {
      getLlmConfiguration: vi.fn(async () => current),
      fetchLlmProviderModels: vi.fn(async () => ({
        providerId: "qwen" as const,
        models: [{
          id: "MiniMax-M2.1", displayName: "MiniMax-M2.1", description: "", isDefault: true,
          supportedReasoningEfforts: [],
        }],
      })),
      saveLlmModelRoutes,
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(<LlmRoutePicker gateway={gateway} onSelection={() => undefined} taskKind="paper_overview" />));
    await act(async () => undefined);
    const inherit = [...container.querySelectorAll<HTMLButtonElement>("button")]
      .find((button) => button.textContent === "恢复继承全局视觉模型")!;

    await act(async () => { inherit.click(); });
    await act(async () => undefined);

    expect(saveLlmModelRoutes).toHaveBeenCalledTimes(1);
    expect(saveLlmModelRoutes).toHaveBeenCalledWith([{
      routeKind: "use_case",
      routeId: "paper_overview",
    }]);
    expect(container.querySelector<HTMLSelectElement>("select")?.value).toBe("qwen");
    expect(container.querySelector<HTMLInputElement>('input[list="local-models-paper_overview"]')?.value)
      .toBe("qwen3-vl-flash-2026-01-22");
  });

  it("ignores a stale cached qwen model list under the openai provider", async () => {
    cacheLlmModels("openai", [{
      id: "qwen3-vl-flash-2026-01-22", displayName: "Qwen stale", description: "", isDefault: true,
      supportedReasoningEfforts: [],
    }]);
    let current: LlmConfiguration = {
      ...configuration,
      providers: [{
        providerId: "openai", displayName: "ChatGPT / Codex", authMode: "chatgpt-login",
        protocol: "codex-cli", modelKinds: ["vision"], defaultBaseUrls: [], baseUrl: "",
        usesDefaultBaseUrl: true, enabled: true, timeoutMs: 120_000, credentialConfigured: false,
      }],
      defaultModels: [{
        routeKind: "default", routeId: "vision", label: "视觉模型", modelKind: "vision",
        selection: { providerId: "openai", modelId: "qwen3-vl-flash-2026-01-22" },
      }],
      useCases: [{
        routeKind: "use_case", routeId: "paper_overview", label: "生成论文概览", modelKind: "vision",
      }],
    };
    const saveLlmModelRoutes = vi.fn(async (drafts: readonly LlmModelRouteDraft[]) => {
      current = {
        ...current,
        useCases: [{ ...current.useCases[0]!, selection: drafts[0]?.selection }],
      } as LlmConfiguration;
      return current;
    });
    const fetchLlmProviderModels = vi.fn(async () => ({
      providerId: "openai" as const,
      models: [{
        id: "gpt-5.6-terra", displayName: "GPT-5.6-Terra", description: "", isDefault: true,
        defaultReasoningEffort: "medium" as const,
        supportedReasoningEfforts: ["medium"] as const,
      }],
    }));
    const gateway = {
      getLlmConfiguration: vi.fn(async () => current),
      fetchLlmProviderModels,
      saveLlmModelRoutes,
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(<LlmRoutePicker gateway={gateway} onSelection={() => undefined} taskKind="paper_overview" />));
    await act(async () => undefined);

    expect(fetchLlmProviderModels).toHaveBeenCalledWith("openai");
    expect(container.querySelector<HTMLInputElement>('input[list="local-models-paper_overview"]')?.value)
      .toBe("gpt-5.6-terra");
  });

  it("reuses a persisted model catalog without fetching it again", async () => {
    cacheLlmModels("qwen", [{
      id: "model-a", displayName: "Model A", description: "", isDefault: true,
      supportedReasoningEfforts: [],
    }]);
    const fetchLlmProviderModels = vi.fn();
    const gateway = {
      getLlmConfiguration: vi.fn(async () => configuration),
      fetchLlmProviderModels,
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(<LlmRoutePicker gateway={gateway} onSelection={() => undefined} taskKind="paper_overview" />));
    await act(async () => undefined);

    expect(fetchLlmProviderModels).not.toHaveBeenCalled();
    const option = container.querySelector<HTMLOptionElement>('datalist option[value="model-a"]');
    expect(option?.textContent).toBe("");
  });
});
