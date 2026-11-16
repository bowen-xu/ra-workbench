// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AiCandidate } from "@research-assistant/core/ai";
import type { LiteratureItem } from "@research-assistant/core/literature";

const notifications = vi.hoisted(() => ({
  error: vi.fn(),
  success: vi.fn(),
  warning: vi.fn(),
}));
vi.mock("../../../../../../apps/web/src/core/services/notifications", () => ({ notify: notifications }));

import type { AiAssistanceGateway } from "../../../../../../apps/web/src/core/services/aiAssistanceGateway";
import {
  PaperConceptMapCandidatePanel,
  paperConceptCandidateStats,
} from "./PaperConceptMapCandidatePanel";

function paper(): LiteratureItem {
  return {
    id: "paper", itemType: "journalArticle", title: "Paper", abstractText: "",
    containerTitle: "", volume: "", issue: "", pages: "", doi: "", url: "",
    language: "", bibtexKey: "", creators: [], tags: [], readingStatus: "unread",
    attachments: [{ id: "attachment", displayName: "paper.pdf", mimeType: "application/pdf",
      byteSize: 10, contentSha256: "a".repeat(64) }],
    projectIds: [], metadataSources: [], createdAt: "now", updatedAt: "now",
  };
}

function candidate(): AiCandidate {
  const concepts = [
    { id: "focal", preferredTerm: "Focal", paperFunction: "focal" as const,
      noveltyStatus: "introduced_by_paper" as const },
    { id: "base", preferredTerm: "Base", paperFunction: "foundational" as const,
      noveltyStatus: "pre_existing" as const },
  ].map((value) => ({ ...value,
    surfaceForms: [{ text: value.preferredTerm, language: "en", relationKind: "preferred" as const,
      evidenceItemIds: ["P1-T1"] }],
    explanationStatus: "explained_in_paper" as const, explanation: `${value.preferredTerm} explanation`,
    disambiguation: "", roleRationale: "key role", noveltyRationale: "source grounded",
    definitionClaims: [], evidenceItemIds: ["P1-T1"], referenceMentions: [],
  }));
  return {
    id: "candidate", runId: "run", providerId: "openai", modelId: "model",
    candidateKind: "paper_concept_map_v1", schemaVersion: 1, currentRevision: 1,
    reviewState: "unreviewed", validationStatus: "valid", originalPayloadHash: "b".repeat(64),
    createdAt: "now", updatedAt: "now",
    evidenceBundle: { id: "bundle", runId: "run", scopeHash: "c".repeat(64),
      retrievalConfigVersion: "paper-context-v1", validationStatus: "valid", createdAt: "now",
      items: [{ id: "evidence", citationKey: "P1-T1", ordinal: 0, anchorId: "anchor",
        paperId: "paper", paperTitle: "Paper", attachmentId: "attachment",
        contentSha256: "a".repeat(64), pageIndex: 0, quoteExact: "grounded quote",
        quotePrefix: "", quoteSuffix: "", contextText: "grounded quote",
        relationKind: "direct_quote", validationStatus: "valid", currentAnchorStatus: "exact",
        createdAt: "now" }],
    },
    revisions: [{ id: "revision", revision: 1, payloadHash: "d".repeat(64), editorKind: "ai",
      sourceRunId: "run", changeReason: "generated", createdAt: "now",
      payload: { kind: "paper_concept_map_v1", schemaVersion: 1, concepts,
        relations: [{ id: "relation", subjectConceptCandidateId: "focal",
          predicateKind: "builds_on", objectConceptCandidateId: "base", scopeNote: "paper scope",
          evidenceItemIds: ["P1-T1"] }], omissions: [] } }],
    decisions: [], attentionFeedback: [], overviewFieldConfirmations: [], summaryVersions: [],
    suggestionItemDecisions: ["focal", "base"].map((id) => ({
      id: `decision-${id}`, candidateId: "candidate", candidateRevision: 1, suggestionId: id,
      action: "accept_unchanged" as const, resolutionKind: "create_concept" as const,
      rationale: "reviewed", idempotencyKey: `key-${id}`, targetObjectKind: "concept" as const,
      targetObjectId: `concept-${id}`, targetObjectRevision: 1, createdAt: "now",
    })),
    citationAssessments: ["focal", "base", "relation"].map((statementId) => ({
      id: `assessment-${statementId}`, candidateId: "candidate", candidateRevision: 1,
      statementId, evidenceItemId: "evidence", assessmentRevision: 1,
      semanticRelation: "supports" as const, rationale: "checked",
      idempotencyKey: `assessment-key-${statementId}`, assessedAt: "now",
    })),
  };
}

describe("paper concept map candidate panel", () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean })
      .IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    vi.clearAllMocks();
  });

  afterEach(async () => {
    await act(async () => root.unmount());
    container.remove();
  });

  it("groups concepts and keeps a grounded relation locked until its source is opened", async () => {
    const recordEvidenceInspection = vi.fn(async () => true);
    const onOpenAttachment = vi.fn();
    await act(async () => root.render(<PaperConceptMapCandidatePanel candidates={[candidate()]}
      gateway={{ recordEvidenceInspection } as unknown as AiAssistanceGateway} item={paper()}
      onCandidatesChanged={vi.fn()} onOpenAttachment={onOpenAttachment} />));

    expect(container.textContent).toContain("核心概念");
    expect(container.textContent).toContain("基础概念");
    expect(container.textContent).toContain("Focal → 基于 → Base");
    const cards = [...container.querySelectorAll<HTMLDetailsElement>(".paper-concept-card")];
    expect(cards).toHaveLength(2);
    expect(cards.every((card) => !card.open)).toBe(true);
    expect(cards[0]?.querySelector(":scope > summary")?.textContent).toContain("Focal");

    const relation = container.querySelector(".paper-concept-relation-card");
    const accept = [...(relation?.querySelectorAll("button") ?? [])]
      .find((button) => button.textContent === "接受为工作知识");
    expect(accept?.disabled).toBe(true);

    await act(async () => {
      relation?.querySelector<HTMLButtonElement>(".ai-citation-source")?.click();
      await Promise.resolve();
    });

    expect(onOpenAttachment).toHaveBeenCalledOnce();
    expect(recordEvidenceInspection).toHaveBeenCalledWith("candidate", 1, "evidence");
    expect(accept?.disabled).toBe(false);
  });

  it("reports the latest concept and pending counts", () => {
    expect(paperConceptCandidateStats([candidate()], "paper"))
      .toEqual({ conceptCount: 2, pendingCount: 0 });
    expect(paperConceptCandidateStats([candidate()], "other-paper")).toBeUndefined();
  });

  it("can discard every pending concept and relation after confirmation", async () => {
    const pending = { ...candidate(), suggestionItemDecisions: [] };
    const decidePaperConceptItem = vi.fn(async (
      _request: Parameters<AiAssistanceGateway["decidePaperConceptItem"]>[0],
    ) => pending);
    await act(async () => root.render(<PaperConceptMapCandidatePanel candidates={[pending]}
      gateway={{ decidePaperConceptItem } as unknown as AiAssistanceGateway} item={paper()}
      onCandidatesChanged={vi.fn()} onOpenAttachment={vi.fn()} />));

    const discard = [...container.querySelectorAll("button")]
      .find((button) => button.textContent === "全部舍弃");
    await act(async () => discard?.click());
    expect(document.body.textContent).toContain("舍弃全部待处理候选？");

    const confirm = [...document.querySelectorAll("button")]
      .find((button) => button.textContent === "舍弃全部待处理项");
    await act(async () => {
      confirm?.click();
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(decidePaperConceptItem).toHaveBeenCalledTimes(3);
    expect(decidePaperConceptItem.mock.calls.map(([request]) => request.action))
      .toEqual(["reject", "reject", "reject"]);
    expect(notifications.success).toHaveBeenCalledWith("已处理 3 条概念候选。");
  });
});
