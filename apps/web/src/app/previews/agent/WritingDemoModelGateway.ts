import type { LlmConfiguration, LlmModelDescriptor } from "@research-assistant/core/ai";
import type { AgentModelGateway } from "../../../core/features/agent/AgentModelControl";

export type WritingDemoModelMode = "default" | "openai";

/** Swap only catalog/configuration data; no alternative picker or network calls. */
export function createWritingDemoModelGateway(): { gateway: AgentModelGateway; setMode: (mode: WritingDemoModelMode) => void } {
  let mode: WritingDemoModelMode = "default";
  const models: readonly LlmModelDescriptor[] = [
    { id: "gpt-5.6-terra", displayName: "GPT-5.6 Terra", description: "模拟 OpenAI 模型", isDefault: true,
      defaultReasoningEffort: "medium", supportedReasoningEfforts: ["low", "medium", "high", "xhigh", "max"], supportsFastMode: true },
    { id: "gpt-5.5", displayName: "GPT-5.5", description: "模拟 OpenAI 模型", isDefault: false,
      defaultReasoningEffort: "medium", supportedReasoningEfforts: ["low", "medium", "high", "xhigh"], supportsFastMode: true },
  ];
  return { setMode: value => { mode = value; }, gateway: {
    getLlmConfiguration: async (): Promise<LlmConfiguration> => {
      const openai = mode === "openai", providerId = openai ? "openai" : "deepseek";
      return { providers: [{ providerId, displayName: "模拟模型（无网络请求）", authMode: openai ? "chatgpt-login" : "api-key",
        protocol: openai ? "codex-cli" : "openai-compatible", modelKinds: ["text"], defaultBaseUrls: [], baseUrl: "",
        usesDefaultBaseUrl: true, enabled: true, timeoutMs: 60000, credentialConfigured: true }],
      defaultModels: [{ routeKind: "default", routeId: "text", label: "文本模型", modelKind: "text",
        selection: openai ? { providerId, modelId: "gpt-5.6-terra", reasoningEffort: "medium" } : { providerId, modelId: "deepseek-v4-pro" } }], useCases: [] };
    },
    fetchLlmProviderModels: async providerId => ({ providerId, models: providerId === "openai" ? models : [
      { id: "deepseek-v4-pro", displayName: "模拟文本模型", description: "只生成模拟稿件修改", isDefault: true, supportedReasoningEfforts: [] },
    ] }),
  } };
}
