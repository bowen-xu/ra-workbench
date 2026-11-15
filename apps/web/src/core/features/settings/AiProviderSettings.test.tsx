/** @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { LlmConfiguration, LlmModelRouteDraft } from "@research-assistant/core/ai";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AiAssistanceGateway } from "../../services/aiAssistanceGateway";
import { cacheLlmModels, readCachedLlmModels } from "../models/llmModelCatalogCache";
import { LLM_CONFIGURATION_CHANGED_EVENT } from "../models/llmConfigurationEvents";
import { AiProviderSettings } from "./AiProviderSettings";

const notifications = vi.hoisted(() => ({
  error: vi.fn(),
  info: vi.fn(),
  success: vi.fn(),
  warning: vi.fn(),
}));

vi.mock("../../services/notifications", () => ({ notify: notifications }));
vi.mock("../../services/codexCatalogVersionGateway", () => ({codexCatalogVersionGateway: {
  getVersions: async () => ({runtimeVersion: "0.146.0", selectedVersion: "0.146.0", availableVersions: ["0.146.0"]}),
}}));

function configuration(configured: boolean): LlmConfiguration {
  return {
    providers: [{
      providerId: "deepseek", displayName: "DeepSeek", authMode: "api-key", protocol: "openai-compatible",
      modelKinds: ["text"], defaultBaseUrls: ["https://api.deepseek.com"],
      baseUrl: "https://api.deepseek.com", usesDefaultBaseUrl: true, enabled: false,
      timeoutMs: 60_000, credentialConfigured: configured,
      ...(configured ? { credentialMask: "••••1234" } : {}),
    }],
    defaultModels: [
      { routeKind: "default", routeId: "text", label: "文本模型", modelKind: "text" },
      { routeKind: "default", routeId: "embedding", label: "Embedding 模型", modelKind: "embedding" },
      { routeKind: "default", routeId: "vision", label: "视觉模型", modelKind: "vision" },
    ],
    useCases: [{ routeKind: "use_case", routeId: "paper_overview", label: "生成论文概览", modelKind: "vision" }],
  };
}

function chatGptConfiguration(): LlmConfiguration {
  return {
    ...configuration(true),
    providers: [{
      providerId: "openai", displayName: "ChatGPT", authMode: "chatgpt-login", protocol: "codex-cli",
      modelKinds: ["text"], defaultBaseUrls: [], baseUrl: "", usesDefaultBaseUrl: false,
      enabled: true, timeoutMs: 120_000, credentialConfigured: true,
    }],
    defaultModels: [{
      routeKind: "default", routeId: "text", label: "文本模型", modelKind: "text",
      selection: { providerId: "openai", modelId: "gpt-5.6-sol", reasoningEffort: "medium" },
    }],
  };
}

function button(label: string): HTMLButtonElement {
  const normalizedLabel = label.replace(/\s+/g, "");
  const match = [...document.body.querySelectorAll("button")]
    .find((item) => item.textContent?.replace(/\s+/g, "").includes(normalizedLabel));
  if (match === undefined) throw new Error(`missing button ${label}`);
  return match;
}

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

describe("floating AI API settings", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    vi.clearAllMocks();
    installMemoryLocalStorage();
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
    document.body.querySelector(".api-settings-backdrop")?.remove();
  });

  it("saves a new key separately and replaces it with a masked summary", async () => {
    cacheLlmModels("deepseek", [{
      id: "stale-model", displayName: "Stale model", description: "", isDefault: true,
      supportedReasoningEfforts: [],
    }]);
    let configured = false;
    const setCredential = vi.fn(async () => {
      configured = true;
      return { providerId: "deepseek" as const, configured: true, maskedValue: "••••1234" };
    });
    const saveLlmProviderConfig = vi.fn(async () => configuration(configured));
    const gateway = {
      getLlmConfiguration: vi.fn(async () => configuration(configured)),
      setCredential,
      saveLlmProviderConfig,
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(<AiProviderSettings desktopAvailable gateway={gateway} onClose={() => undefined} />));
    await act(async () => undefined);

    const keyInput = document.body.querySelector<HTMLInputElement>('.api-key-control input[type="password"]');
    expect(keyInput).not.toBeNull();
    await act(async () => {
      const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      setValue?.call(keyInput, "sk-secret-1234");
      keyInput?.dispatchEvent(new Event("input", { bubbles: true }));
    });
    const reveal = document.body.querySelector<HTMLButtonElement>('button[aria-label="显示 API key"]');
    expect(reveal?.disabled).toBe(false);
    await act(async () => reveal?.click());
    expect(keyInput?.type).toBe("text");
    expect(keyInput?.value).toBe("sk-secret-1234");
    const mask = document.body.querySelector<HTMLButtonElement>('button[aria-label="隐藏 API key"]');
    await act(async () => mask?.click());
    expect(keyInput?.type).toBe("password");
    expect(keyInput?.value).toBe("sk-secret-1234");
    await act(async () => button("保存 Provider").click());

    expect(setCredential).toHaveBeenCalledWith({ providerId: "deepseek", apiKey: "sk-secret-1234" });
    expect(saveLlmProviderConfig).toHaveBeenCalledWith({
      providerId: "deepseek",
      enabled: false,
      baseUrl: "",
      timeoutMs: 60_000,
    });
    expect(document.body.textContent).not.toContain("sk-secret-1234");
    expect(document.body.querySelector<HTMLInputElement>(".api-key-control input")?.value).toBe("••••1234");
    expect(document.body.querySelector('button[aria-label="显示 API key"]')).toBeNull();
    expect(notifications.success).toHaveBeenCalledWith("API key 与 Provider 设置已保存。");
    expect(document.body.querySelector(".api-provider-message")).toBeNull();
    expect(readCachedLlmModels("deepseek")).toBeUndefined();
  });

  it("closes with its accessible close control inside the shared modal", async () => {
    const onClose = vi.fn();
    const gateway = {
      getLlmConfiguration: vi.fn(async () => configuration(true)),
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(<AiProviderSettings desktopAvailable gateway={gateway} onClose={onClose} />));
    await act(async () => undefined);
    const close = document.querySelector<HTMLButtonElement>('[aria-label="关闭 API 设置"]')!;
    expect(close.closest('[data-overlay-scope="modal"]')).not.toBeNull();
    await act(async () => close.click());
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("loads the native account catalog with compact ID-only options and left-aligned actions", async () => {
    const models = [{
      id: "gpt-5.6-sol", displayName: "GPT-5.6-Sol", description: "Frontier model",
      isDefault: true, defaultReasoningEffort: "medium", supportedReasoningEfforts: ["low", "medium"],
    }];
    const fetchLlmProviderModels = vi.fn(async () => ({ providerId: "openai", models }));
    const gateway = {
      getLlmConfiguration: vi.fn(async () => chatGptConfiguration()),
      getChatGptAccountStatus: vi.fn(async () => ({
        runtimeAvailable: true, configured: true, authType: "chatgpt", email: "reader@example.com",
      })),
      fetchLlmProviderModels,
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(<AiProviderSettings desktopAvailable gateway={gateway} onClose={() => undefined} />));
    await act(async () => undefined);

    const panel = [...document.body.querySelectorAll<HTMLElement>(".api-provider-panel")]
      .find((candidate) => candidate.querySelector("h3")?.textContent === "ChatGPT");
    expect(panel?.textContent).toContain("1 个模型");
    const actions = [...(panel?.querySelectorAll<HTMLButtonElement>(".api-provider-actions button") ?? [])];
    expect(actions.map((action) => action.textContent?.trim())).toEqual(["退出登录", "刷新模型列表"]);
    expect(actions[0]?.classList.contains("danger-primary")).toBe(true);
    expect(panel?.textContent).toContain("仅用于登录与账号状态");
    const modelOption = document.body.querySelector<HTMLOptionElement>('datalist option[value="gpt-5.6-sol"]');
    expect(modelOption?.textContent).toBe("");
    const routeRow = document.body.querySelector<HTMLElement>(".api-route-row-reasoning");
    expect(routeRow?.querySelectorAll(":scope > select, :scope > input")).toHaveLength(3);
    expect(fetchLlmProviderModels).toHaveBeenCalledWith("openai");
  });

  it("shows an honest loading state while checking the ChatGPT runtime", async () => {
    let finish!: (status: {
      runtimeAvailable: boolean;
      configured: boolean;
      authType: string;
    }) => void;
    const pending = new Promise<{ runtimeAvailable: boolean; configured: boolean; authType: string }>((resolve) => {
      finish = resolve;
    });
    const gateway = {
      getLlmConfiguration: vi.fn(async () => chatGptConfiguration()),
      getChatGptAccountStatus: vi.fn(() => pending),
      fetchLlmProviderModels: vi.fn(async () => ({ providerId: "openai", models: [] })),
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(<AiProviderSettings desktopAvailable gateway={gateway} onClose={() => undefined} />));
    await act(async () => undefined);

    const panel = [...document.body.querySelectorAll<HTMLElement>(".api-provider-panel")]
      .find((candidate) => candidate.querySelector("h3")?.textContent === "ChatGPT");
    expect(panel?.textContent).toContain("检查中");
    expect(panel?.textContent).toContain("正在检查内置运行时与账号状态");
    expect(panel?.textContent).not.toContain("内置 Codex 运行时不可用");
    expect(panel?.querySelector<HTMLButtonElement>(".api-provider-actions button")?.disabled).toBe(true);

    await act(async () => finish({ runtimeAvailable: true, configured: true, authType: "chatgpt" }));
    expect(panel?.textContent).toContain("已登录");
    expect(panel?.textContent).not.toContain("正在检查内置运行时与账号状态");
  });

  it("offers login when the remembered account has no usable managed credential", async () => {
    const startChatGptLogin = vi.fn();
    const gateway = {
      getLlmConfiguration: vi.fn(async () => chatGptConfiguration()),
      getChatGptAccountStatus: vi.fn(async () => ({
        runtimeAvailable: true, configured: false, authType: "chatgpt", email: "remembered@example.com",
      })),
      startChatGptLogin,
      fetchLlmProviderModels: vi.fn(),
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(<AiProviderSettings desktopAvailable gateway={gateway} onClose={() => undefined} />));
    await act(async () => undefined);

    const panel = [...document.body.querySelectorAll<HTMLElement>(".api-provider-panel")]
      .find((candidate) => candidate.querySelector("h3")?.textContent === "ChatGPT");
    expect(panel?.textContent).toContain("未登录");
    expect(panel?.textContent).toContain("remembered@example.com");
    expect(panel?.querySelector(".api-provider-actions button")?.textContent).toContain("登录 OpenAI / ChatGPT");
    expect(panel?.textContent).not.toContain("退出登录");
  });

  it("forces native ChatGPT refresh and leaves account caching to the native boundary", async () => {
    cacheLlmModels("openai", [{
      id: "old-model", displayName: "Old model", description: "", isDefault: true,
      supportedReasoningEfforts: [],
    }]);
    let loggedIn = true;
    const gateway = {
      getLlmConfiguration: vi.fn(async () => chatGptConfiguration()),
      getChatGptAccountStatus: vi.fn(async () => ({
        runtimeAvailable: true, configured: loggedIn, authType: loggedIn ? "chatgpt" : undefined,
      })),
      fetchLlmProviderModels: vi.fn(async () => ({
        providerId: "openai" as const,
        models: [{
          id: "new-model", displayName: "New model", description: "", isDefault: true,
          supportedReasoningEfforts: [],
        }],
      })),
      logoutChatGpt: vi.fn(async () => { loggedIn = false; }),
    } as unknown as AiAssistanceGateway;
    Object.defineProperty(window, "confirm", { configurable: true, value: vi.fn(() => true) });

    await act(async () => root.render(<AiProviderSettings desktopAvailable gateway={gateway} onClose={() => undefined} />));
    await act(async () => undefined);
    const changed = vi.fn();
    window.addEventListener(LLM_CONFIGURATION_CHANGED_EVENT, changed, { once: true });
    await act(async () => button("刷新模型列表").click());
    expect(gateway.fetchLlmProviderModels).toHaveBeenCalledWith("openai", true);
    expect(changed).toHaveBeenCalledOnce();
    expect(readCachedLlmModels("openai")).toBeUndefined();

    await act(async () => button("退出登录").click());
    expect(gateway.logoutChatGpt).toHaveBeenCalledOnce();
    expect(readCachedLlmModels("openai")).toBeUndefined();
  });

  it("marks disabled route providers and blocks saving them as defaults", async () => {
    const qwenDisabled: LlmConfiguration = {
      ...configuration(true),
      providers: [
        {
          providerId: "qwen", displayName: "Qwen", authMode: "api-key", protocol: "openai-compatible",
          modelKinds: ["text", "embedding", "vision"], defaultBaseUrls: ["https://dashscope.aliyuncs.com/compatible-mode/v1"],
          baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1", usesDefaultBaseUrl: true,
          enabled: false, timeoutMs: 60_000, credentialConfigured: true, credentialMask: "••••itJn",
        },
        {
          providerId: "openai", displayName: "ChatGPT / Codex", authMode: "chatgpt-login", protocol: "codex-cli",
          modelKinds: ["text", "vision"], defaultBaseUrls: [], baseUrl: "", usesDefaultBaseUrl: true,
          enabled: true, timeoutMs: 120_000, credentialConfigured: true,
        },
      ],
      defaultModels: [
        { routeKind: "default", routeId: "text", label: "文本模型", modelKind: "text" },
        { routeKind: "default", routeId: "embedding", label: "Embedding 模型", modelKind: "embedding" },
        {
          routeKind: "default", routeId: "vision", label: "视觉模型", modelKind: "vision",
          selection: { providerId: "qwen", modelId: "qwen3-vl-flash-2026-01-22" },
        },
      ],
      useCases: [],
    };
    const saveLlmModelRoutes = vi.fn(async (_drafts: readonly LlmModelRouteDraft[]) => qwenDisabled);
    const gateway = {
      getLlmConfiguration: vi.fn(async () => qwenDisabled),
      getChatGptAccountStatus: vi.fn(async () => ({
        runtimeAvailable: true, configured: true, authType: "chatgpt",
      })),
      saveLlmModelRoutes,
      fetchLlmProviderModels: vi.fn(async () => ({ providerId: "openai", models: [] })),
    } as unknown as AiAssistanceGateway;

    await act(async () => root.render(<AiProviderSettings desktopAvailable gateway={gateway} onClose={() => undefined} />));
    await act(async () => undefined);

    const option = [...document.body.querySelectorAll<HTMLOptionElement>("option")]
      .find((candidate) => candidate.textContent === "Qwen（未启用）");
    expect(option?.disabled).toBe(true);
    expect(document.body.querySelector<HTMLInputElement>('input[aria-label="视觉模型 模型"]')?.disabled).toBe(true);

    await act(async () => button("保存分配").click());

    expect(saveLlmModelRoutes).not.toHaveBeenCalled();
    expect(notifications.error).toHaveBeenCalledWith("请先启用 Qwen 并确认凭据可用，再保存“视觉模型”。");
  });

  it("uses the shared warning when refresh falls back to the account cache", async () => {
    const gateway = {
      getLlmConfiguration: vi.fn(async () => chatGptConfiguration()),
      getChatGptAccountStatus: vi.fn(async () => ({ runtimeAvailable: true, configured: true, authType: "chatgpt" })),
      fetchLlmProviderModels: vi.fn(async () => ({ providerId: "openai", models: [], catalog: { source: "cache", stale: true, fetchedAt: 123 } })),
    } as unknown as AiAssistanceGateway;
    await act(async () => root.render(<AiProviderSettings desktopAvailable gateway={gateway} onClose={() => undefined} />));
    await act(async () => button("刷新模型列表").click());
    expect(gateway.fetchLlmProviderModels).toHaveBeenCalledWith("openai", true);
    expect(notifications.warning).toHaveBeenCalledWith("暂时无法刷新，正在使用该账号上次成功获取的模型目录。");
    expect(notifications.success).not.toHaveBeenCalled();
  });
});
