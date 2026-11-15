import { describe, expect, it } from "vitest";

import { decodePaperReadingBootstrapOptions } from "./paperReadingInteractionDecoders";

const structuredHistory = {
  workflowId: "workflow-1",
  workflowRevision: 2,
  workflowStatus: "needs_review",
  providerId: "qwen",
  modelId: "qwen-vl-max",
  completedTurns: ["paper_overview", "paper_concept_map"],
  attachmentCount: 1,
  updatedAt: "2026-08-09T00:00:00.000Z",
};

describe("paper reading interaction IPC decoders", () => {
  it("decodes fresh and structured bootstrap choices", () => {
    expect(decodePaperReadingBootstrapOptions({
      defaultKind: "fresh_paper",
      unavailableReason: "尚无可复用的结构化阅读历史",
    })).toEqual({
      defaultKind: "fresh_paper",
      unavailableReason: "尚无可复用的结构化阅读历史",
    });
    expect(decodePaperReadingBootstrapOptions({
      defaultKind: "structured_history",
      structuredHistory,
    })).toEqual({
      defaultKind: "structured_history",
      structuredHistory,
    });
  });

  it("rejects incomplete or internally inconsistent structured history", () => {
    expect(decodePaperReadingBootstrapOptions({
      defaultKind: "structured_history",
    })).toBeUndefined();
    expect(decodePaperReadingBootstrapOptions({
      defaultKind: "structured_history",
      structuredHistory: { ...structuredHistory, completedTurns: ["paper_concept_map"] },
    })).toBeUndefined();
    expect(decodePaperReadingBootstrapOptions({
      defaultKind: "structured_history",
      structuredHistory: { ...structuredHistory, workflowRevision: 0 },
    })).toBeUndefined();
  });
});
