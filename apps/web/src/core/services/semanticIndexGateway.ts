import { resolveAppGateway } from "../runtime/appRuntime";
import { invoke, isDesktopRuntime } from "../runtime/platformCore";
import type {
  EmbeddingJobSummary,
  SemanticIndexGateway,
  SemanticIndexGenerationSummary,
  SemanticIndexReconciliationSummary,
  SemanticIndexStatus,
} from "@research-assistant/data";

import {
  decodeEmbeddingJobSummary,
  decodeSemanticIndexGenerationSummary,
  decodeSemanticIndexReconciliationSummary,
  decodeSemanticIndexStatus,
} from "./semanticIndexDecoders";

async function decoded<T>(
  command: string,
  args: Record<string, unknown>,
  decode: (value: unknown) => T | undefined,
): Promise<T> {
  const value = await invoke<unknown>(command, args);
  const result = decode(value);
  if (result === undefined) {
    throw new Error("桌面核心返回了不兼容的语义索引数据；本次结果未被采用。");
  }
  return result;
}

const desktopGateway: SemanticIndexGateway = {
  getStatus: () => decoded("get_semantic_index_status", {}, decodeSemanticIndexStatus),
  getJob: (jobId) => decoded(
    "get_semantic_embedding_job", { jobId }, decodeEmbeddingJobSummary,
  ),
  retryRepresentation: (representationId) => decoded(
    "retry_semantic_representation", { representationId }, decodeEmbeddingJobSummary,
  ),
  cancelJob: (jobId) => invoke<boolean>("cancel_semantic_index_job", { jobId }),
  reconcile: () => decoded(
    "run_semantic_index_reconciliation", {}, decodeSemanticIndexReconciliationSummary,
  ),
  rebuild: (embeddingSpaceId) => decoded(
    "rebuild_semantic_index", { embeddingSpaceId }, decodeSemanticIndexGenerationSummary,
  ),
};

const browserGateway: SemanticIndexGateway = {
  async getStatus(): Promise<SemanticIndexStatus> {
    return { spaces: [], runningJobIds: [], reconciliationRequired: false };
  },
  async getJob(): Promise<EmbeddingJobSummary> {
    throw new Error("浏览器预览不执行本地语义索引任务。");
  },
  async retryRepresentation(): Promise<EmbeddingJobSummary> {
    throw new Error("浏览器预览不执行本地语义索引任务。");
  },
  async cancelJob() { return false; },
  async reconcile(): Promise<SemanticIndexReconciliationSummary> {
    return {
      runId: "browser-preview", status: "succeeded", checkedCount: 0,
      missingCount: 0, orphanCount: 0, repairedCount: 0,
      startedAt: new Date(0).toISOString(), finishedAt: new Date(0).toISOString(),
    };
  },
  async rebuild(embeddingSpaceId: string): Promise<SemanticIndexGenerationSummary> {
    return {
      generationId: "browser-preview", embeddingSpaceId, namespace: "accepted_concepts",
      generation: 1, status: "active", recordCount: 0, readyCount: 0,
      createdAt: new Date(0).toISOString(), activatedAt: new Date(0).toISOString(),
    };
  },
};

export function getSemanticIndexGateway(): SemanticIndexGateway {
  return resolveAppGateway("getSemanticIndexGateway", isDesktopRuntime() ? desktopGateway : browserGateway);
}
