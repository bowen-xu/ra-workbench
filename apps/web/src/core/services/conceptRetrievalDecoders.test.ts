import { describe, expect, it } from "vitest";

import { decodeConceptRetrievalPreview } from "./conceptRetrievalDecoders";

function preview(): Record<string, unknown> {
  return {
    traceId: "trace-1",
    semanticStatus: "used",
    embeddingSpaceId: "space-1",
    indexGenerationId: "generation-1",
    seeds: [{
      conceptId: "concept-1",
      conceptRevision: 2,
      label: "Graph retrieval",
      preferredTerm: "graph retrieval",
      matchedBy: ["lexical", "vector"],
      lexicalRank: 1,
      vectorRank: 2,
      vectorDistance: 0.2,
      rrfScore: 0.03,
    }],
    edges: [],
    sources: [{
      anchorId: "anchor-1",
      conceptId: "concept-1",
      conceptLabel: "Graph retrieval",
      paperId: "paper-2",
      paperTitle: "Prior paper",
      sourceKind: "definition_claim",
      sourceSubjectId: "definition-1",
      sourceSubjectRevision: 1,
      pageIndex: 3,
      quoteExact: "Quoted source",
      contextText: "Definition",
      selected: true,
    }],
    selectedSourceCount: 1,
    externalPaperCount: 1,
    contextSha256: "a".repeat(64),
    expiresAt: "2026-08-11T12:15:00.000Z",
  };
}

describe("concept retrieval IPC decoder", () => {
  it("accepts an auditable preview and rejects inconsistent source counts", () => {
    expect(decodeConceptRetrievalPreview(preview())?.traceId).toBe("trace-1");
    expect(decodeConceptRetrievalPreview({ ...preview(), selectedSourceCount: 0 }))
      .toBeUndefined();
  });

  it("requires embedding space and generation identifiers as a pair", () => {
    const value = preview();
    delete value.indexGenerationId;
    expect(decodeConceptRetrievalPreview(value)).toBeUndefined();
  });

  it("rejects unsupported retrieval and source enum values", () => {
    expect(decodeConceptRetrievalPreview({ ...preview(), semanticStatus: "approximate" }))
      .toBeUndefined();
    const value = preview();
    value.sources = [{ ...(value.sources as Record<string, unknown>[])[0], sourceKind: "paper" }];
    expect(decodeConceptRetrievalPreview(value)).toBeUndefined();
  });
});
