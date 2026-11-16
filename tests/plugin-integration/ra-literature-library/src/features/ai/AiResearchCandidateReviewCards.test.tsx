import { renderToStaticMarkup } from "react-dom/server";
import type { AiCandidate } from "@research-assistant/core/ai";
import { describe, expect, it } from "vitest";

import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/services/aiAssistanceGateway";
import { AiDerivedResearchViewReviewCard } from "./AiDerivedResearchViewReviewCard";
import { AiReferenceRecordReviewCard } from "./AiReferenceRecordReviewCard";

const gateway = {} as AiAssistanceGateway;
const baseCandidate = {
  id: "candidate",
  runId: "run",
  currentRevision: 2,
  reviewState: "edited",
  providerId: "fixture-generation",
  modelId: "fixture-v1",
  schemaVersion: 1,
  validationStatus: "valid",
  originalPayloadHash: "initial-hash",
  createdAt: "2026-08-04T00:00:00Z",
  updatedAt: "2026-08-04T00:01:00Z",
  evidenceBundle: {
    id: "bundle",
    runId: "run",
    scopeHash: "scope",
    retrievalConfigVersion: "research-v1",
    validationStatus: "valid",
    createdAt: "2026-08-04T00:00:00Z",
    items: [],
  },
  decisions: [],
  attentionFeedback: [],
  suggestionItemDecisions: [],
  citationAssessments: [],
  overviewFieldConfirmations: [],
} as const;

describe("research candidate review revisions", () => {
  it("shows the immutable AI reference draft beside the human revision", () => {
    const candidate: AiCandidate = {
      ...baseCandidate,
      candidateKind: "reference_record_candidate_v1",
      revisions: [
        {
          id: "revision-1",
          revision: 1,
          editorKind: "ai",
          payloadHash: "ai-hash",
          changeReason: "initial_generation",
          createdAt: "2026-08-04T00:00:00Z",
          payload: {
            kind: "reference_record_candidate_v1",
            schemaVersion: 1,
            rawText: "AI raw reference",
            fields: [{ name: "title", value: "AI title", evidenceItemIds: ["E1"] }],
            evidenceItemIds: ["E1"],
            uncertainties: [],
          },
        },
        {
          id: "revision-2",
          revision: 2,
          editorKind: "human",
          payloadHash: "human-hash",
          parentRevision: 1,
          changeReason: "human_reference_field_review",
          createdAt: "2026-08-04T00:01:00Z",
          payload: {
            kind: "reference_record_candidate_v1",
            schemaVersion: 1,
            rawText: "Human corrected reference",
            fields: [{ name: "title", value: "Human title", evidenceItemIds: ["E1"] }],
            evidenceItemIds: ["E1"],
            uncertainties: ["Check edition"],
          },
        },
      ],
    };

    const html = renderToStaticMarkup(<AiReferenceRecordReviewCard
      candidate={candidate}
      gateway={gateway}
      onChanged={() => undefined}
      onOpenEvidence={() => Promise.resolve()}
    />);
    expect(html).toContain("AI raw reference");
    expect(html).toContain("Human corrected reference");
    expect(html).toContain("AI title");
    expect(html).toContain("Human title");
  });

  it("shows the immutable AI synthesis beside the human revision", () => {
    const candidate: AiCandidate = {
      ...baseCandidate,
      candidateKind: "derived_research_view_v1",
      revisions: [
        {
          id: "revision-1",
          revision: 1,
          editorKind: "ai",
          payloadHash: "ai-hash",
          changeReason: "initial_generation",
          createdAt: "2026-08-04T00:00:00Z",
          payload: {
            kind: "derived_research_view_v1",
            schemaVersion: 1,
            viewKind: "taxonomy",
            title: "AI taxonomy",
            scopeNote: "AI scope",
            items: [{ id: "item_ai", text: "AI synthesis", epistemicRole: "synthesis", evidenceItemIds: ["E1", "E2"], hasConflict: false }],
          },
        },
        {
          id: "revision-2",
          revision: 2,
          editorKind: "human",
          payloadHash: "human-hash",
          parentRevision: 1,
          changeReason: "human_derived_view_review",
          createdAt: "2026-08-04T00:01:00Z",
          payload: {
            kind: "derived_research_view_v1",
            schemaVersion: 1,
            viewKind: "taxonomy",
            title: "Human taxonomy",
            scopeNote: "Human scope",
            items: [{ id: "item_ai", text: "Human synthesis", epistemicRole: "inference", evidenceItemIds: ["E1"], hasConflict: true }],
          },
        },
      ],
    };

    const html = renderToStaticMarkup(<AiDerivedResearchViewReviewCard
      candidate={candidate}
      gateway={gateway}
      onChanged={() => undefined}
      onOpenEvidence={() => Promise.resolve()}
      projectId="project"
    />);
    expect(html).toContain("AI synthesis");
    expect(html).toContain("Human synthesis");
    expect(html).toContain("初始 AI 原稿");
    expect(html).toContain("当前人工 revision");
  });
});
