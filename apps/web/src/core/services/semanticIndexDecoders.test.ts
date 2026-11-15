import { describe, expect, it } from "vitest";

import {
  decodeEmbeddingJobSummary,
  decodeSemanticIndexGenerationSummary,
  decodeSemanticIndexReconciliationSummary,
  decodeSemanticIndexStatus,
} from "./semanticIndexDecoders";

describe("semantic index IPC decoders", () => {
  it("decodes stable job and control-plane status contracts", () => {
    expect(decodeEmbeddingJobSummary({
      jobId: "job-1", representationId: "representation-1", subjectKind: "concept",
      subjectId: "concept-1", status: "queued", attempt: 2,
      createdAt: "2026-08-11T00:00:00Z",
    })).toEqual(expect.objectContaining({ jobId: "job-1", attempt: 2 }));
    expect(decodeSemanticIndexStatus({
      spaces: [{
        embeddingSpaceId: "space-1", providerId: "qwen-embedding",
        modelId: "text-embedding-v4", modelRevision: "v4", dimension: 1024,
        generation: 1, generationStatus: "active", namespace: "accepted_concepts",
        readyCount: 3, pendingCount: 1, failedCount: 0, staleCount: 0,
      }],
      runningJobIds: ["job-1"], reconciliationRequired: false,
    })?.spaces).toHaveLength(1);
    expect(decodeSemanticIndexReconciliationSummary({
      runId: "run-1", status: "succeeded", checkedCount: 4, missingCount: 1,
      orphanCount: 1, repairedCount: 3, startedAt: "start", finishedAt: "finish",
    })).toEqual(expect.objectContaining({ checkedCount: 4, repairedCount: 3 }));
    expect(decodeSemanticIndexGenerationSummary({
      generationId: "generation-2", embeddingSpaceId: "space-1",
      namespace: "accepted_concepts", generation: 2, status: "building",
      recordCount: 4, readyCount: 1, createdAt: "start",
    })).toEqual(expect.objectContaining({ generation: 2, recordCount: 4 }));
  });

  it("rejects a bad job but preserves valid rows in best-effort status lists", () => {
    expect(decodeEmbeddingJobSummary({
      jobId: "job-1", representationId: "representation-1", subjectKind: "concept",
      subjectId: "concept-1", status: "unknown", attempt: 0, createdAt: "now",
    })).toBeUndefined();
    const decoded = decodeSemanticIndexStatus({
      spaces: [
        { generationStatus: "unknown" },
        {
          embeddingSpaceId: "space-1", providerId: "qwen-embedding",
          modelId: "text-embedding-v4", modelRevision: "v4", dimension: 1024,
          generation: 1, generationStatus: "building", namespace: "accepted_concepts",
          readyCount: 0, pendingCount: 1, failedCount: 0, staleCount: 0,
        },
      ],
      runningJobIds: [null, "job-1"], reconciliationRequired: false,
    });
    expect(decoded?.spaces).toHaveLength(1);
    expect(decoded?.runningJobIds).toEqual(["job-1"]);
  });
});
