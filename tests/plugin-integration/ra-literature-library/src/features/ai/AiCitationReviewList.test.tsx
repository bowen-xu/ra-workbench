/** @vitest-environment happy-dom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import type { AiCandidate } from "@research-assistant/core/ai";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/services/aiAssistanceGateway";
import {
  AiCitationReviewList,
  isCitationTargetReady,
} from "./AiCitationReviewList";

function candidate(): AiCandidate {
  return {
    id: "candidate", runId: "run", providerId: "fixture-generation", modelId: "fixture-v1",
    candidateKind: "selection_explanation_v1", schemaVersion: 1, currentRevision: 1,
    reviewState: "unreviewed", validationStatus: "valid", originalPayloadHash: "a".repeat(64),
    createdAt: "now", updatedAt: "now", decisions: [], attentionFeedback: [],
    suggestionItemDecisions: [], citationAssessments: [],
    overviewFieldConfirmations: [],
    evidenceBundle: {
      id: "bundle", runId: "run", scopeHash: "b".repeat(64),
      retrievalConfigVersion: "selection-v1", validationStatus: "valid", createdAt: "now",
      items: [{
        id: "evidence", citationKey: "ev-quote", ordinal: 0, anchorId: "anchor",
        paperId: "paper", paperTitle: "Paper", attachmentId: "attachment",
        contentSha256: "c".repeat(64), pageIndex: 1, quoteExact: "Exact quote",
        quotePrefix: "", quoteSuffix: "", contextText: "Exact quote",
        relationKind: "direct_quote", validationStatus: "valid",
        currentAnchorStatus: "page_level", createdAt: "now",
      }],
    },
    revisions: [{
      id: "revision", revision: 1, payloadHash: "a".repeat(64), editorKind: "ai",
      sourceRunId: "run", changeReason: "provider_generation", createdAt: "now",
      payload: {
        kind: "selection_explanation_v1", schemaVersion: 1,
        statements: [{
          id: "s1", text: "Explanation", epistemicRole: "source_paraphrase",
          evidenceItemIds: ["ev-quote"],
        }],
        uncertainties: [],
      },
    }],
  };
}

describe("human citation assessment", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it("shows anchor degradation and saves a separate human semantic judgment", async () => {
    const initial = candidate();
    const recordCitationAssessment = vi.fn(async (draft) => ({
      ...initial,
      citationAssessments: [{
        id: "assessment", candidateId: initial.id, candidateRevision: 1,
        statementId: draft.statementId, evidenceItemId: draft.evidenceItemId,
        assessmentRevision: 1, semanticRelation: draft.semanticRelation,
        rationale: draft.rationale, idempotencyKey: draft.idempotencyKey, assessedAt: "now",
      }],
    } satisfies AiCandidate));
    const onChanged = vi.fn();
    const gateway = { recordCitationAssessment } as unknown as AiAssistanceGateway;
    await act(async () => root.render(<AiCitationReviewList candidate={initial}
      evidenceItemIds={["ev-quote"]} gateway={gateway} onChanged={onChanged}
      onOpenEvidence={() => Promise.resolve()} statementId="s1" />));

    expect(container.textContent).toContain("页级定位（已降级）");
    expect(container.textContent).toContain("人工语义：未审查");
    const select = container.querySelector("select");
    await act(async () => {
      if (select === null) return;
      select.value = "supports";
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    const save = [...container.querySelectorAll("button")]
      .find((button) => button.textContent === "保存判断");
    await act(async () => save?.dispatchEvent(new MouseEvent("click", { bubbles: true })));

    expect(recordCitationAssessment).toHaveBeenCalledWith(expect.objectContaining({
      candidateId: "candidate", candidateRevision: 1, statementId: "s1",
      evidenceItemId: "evidence", expectedAssessmentRevision: 0,
      semanticRelation: "supports",
    }));
    expect(onChanged).toHaveBeenCalledWith(expect.anything(), false);
  });

  it("requires a positive latest judgment and rejects stale or negative citations", () => {
    const initial = candidate();
    const supported: AiCandidate = {
      ...initial,
      citationAssessments: [{
        id: "assessment", candidateId: initial.id, candidateRevision: 1,
        statementId: "s1", evidenceItemId: "evidence", assessmentRevision: 1,
        semanticRelation: "supports", rationale: "checked", idempotencyKey: "key",
        assessedAt: "now",
      }],
    };
    expect(isCitationTargetReady(supported, "s1", ["ev-quote"])).toBe(true);
    expect(isCitationTargetReady({
      ...supported,
      evidenceBundle: {
        ...supported.evidenceBundle,
        items: supported.evidenceBundle.items.map((item) => ({
          ...item, currentAnchorStatus: "stale" as const,
        })),
      },
    }, "s1", ["ev-quote"])).toBe(false);
    expect(isCitationTargetReady({
      ...supported,
      citationAssessments: [{
        ...supported.citationAssessments[0]!, semanticRelation: "unsupported",
      }],
    }, "s1", ["ev-quote"])).toBe(false);
  });
});
