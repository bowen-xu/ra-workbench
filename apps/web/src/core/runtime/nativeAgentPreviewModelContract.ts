import { executionModeForModel } from "@research-assistant/core/agent";
import { decodeLlmProviderModels } from "../services/aiDecoders";
import type { NativeAgentPreview } from "./nativeAgentPreview";

/** The harness may replace model data, while the production admission profile
 * remains authoritative. Check the real edge DTO before mounting its host. */
export async function checkNativeAgentPreviewModelContract(preview: Pick<NativeAgentPreview, "bootstrap" | "invoke">): Promise<void> {
  const model = preview.bootstrap.model;
  const catalog = decodeLlmProviderModels(await preview.invoke("fetch_llm_provider_models", { providerId: model.providerId, refresh: false }));
  if (!catalog || catalog.providerId !== model.providerId || executionModeForModel(model) === null
    || catalog.models.filter(candidate => candidate.id === model.modelId).length !== 1
    || catalog.models.some(candidate => {
      const identity = { providerId: catalog.providerId, modelId: candidate.id };
      return executionModeForModel(identity, candidate.defaultReasoningEffort) === null
        || candidate.supportedReasoningEfforts.some(effort => executionModeForModel(identity, effort) === null)
        || (candidate.defaultReasoningEffort !== undefined && !candidate.supportedReasoningEfforts.includes(candidate.defaultReasoningEffort));
    })) throw new Error("agent_preview_model_contract_invalid: 合成模型与正式执行配置不兼容。");
}
